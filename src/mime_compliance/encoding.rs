// Adapted from MimeKit's Encodings/IEncodingValidator.cs and encoding validators.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io;

use super::{
    MimeComplianceContext, MimeComplianceIssue, MimeComplianceLogger, MimeComplianceOptions,
    MimeCompliancePositionKind, MimeComplianceValidators as Validators,
    MimeComplianceViolation as Violation,
};
use crate::{ContentEncoding, MimePosition};

mod base64;
mod quoted_printable;
mod uu;

enum State {
    Base64(base64::Base64),
    QuotedPrintable(quoted_printable::QuotedPrintable),
    Uu(uu::Uu),
}

pub(super) struct EncodingValidator {
    state: State,
    position: Position,
}

impl EncodingValidator {
    pub(super) fn new(
        encoding: ContentEncoding,
        options: MimeComplianceOptions,
        position: MimePosition,
    ) -> Option<Self> {
        let enabled = options.enabled_validators;
        let state = match encoding {
            ContentEncoding::Base64 if enabled.contains(Validators::BASE64) => {
                State::Base64(base64::Base64::default())
            }
            ContentEncoding::QuotedPrintable if enabled.contains(Validators::QUOTED_PRINTABLE) => {
                State::QuotedPrintable(quoted_printable::QuotedPrintable::default())
            }
            ContentEncoding::UuEncode if enabled.contains(Validators::UU_ENCODE) => {
                State::Uu(uu::Uu::default())
            }
            _ => return None,
        };
        Some(Self {
            state,
            position: Position {
                context: options.context,
                offset: position.offset,
                line_start: position.offset,
                line: position.line,
            },
        })
    }

    pub(super) fn write(
        &mut self,
        bytes: &[u8],
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        match &mut self.state {
            State::Base64(state) => state.write(bytes, &mut self.position, logger),
            State::QuotedPrintable(state) => state.write(bytes, &mut self.position, logger),
            State::Uu(state) => state.write(bytes, &mut self.position, logger),
        }
    }

    pub(super) fn finish(&self, logger: &mut dyn MimeComplianceLogger) -> io::Result<()> {
        match &self.state {
            State::Base64(state) => state.finish(&self.position, logger),
            State::QuotedPrintable(state) => state.finish(&self.position, logger),
            State::Uu(state) => state.finish(&self.position, logger),
        }
    }
}

struct Position {
    context: MimeComplianceContext,
    offset: u64,
    line_start: u64,
    line: u64,
}

impl Position {
    fn advance(&mut self, byte: u8) {
        self.offset += 1;
        if byte == b'\n' {
            self.line += 1;
            self.line_start = self.offset;
        }
    }

    fn log(&self, logger: &mut dyn MimeComplianceLogger, violation: Violation) -> io::Result<()> {
        self.log_at(logger, violation, self.offset)
    }

    fn log_at(
        &self,
        logger: &mut dyn MimeComplianceLogger,
        violation: Violation,
        offset: u64,
    ) -> io::Result<()> {
        // Track only the start of the current line. Compute the column from the
        // reported offset, since a defect need not be at the current cursor.
        logger.log(MimeComplianceIssue {
            context: self.context,
            violation,
            stream_offset: offset,
            line_number: self.line,
            column_number: offset + 1 - self.line_start,
            position_kind: MimeCompliancePositionKind::Exact,
        })
    }
}

#[cfg(test)]
mod tests;

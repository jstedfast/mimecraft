// Adapted from MimeKit's Encodings/Base64Validator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{MimeComplianceLogger, Position, Violation, io};

const ALPHABET: u8 = 0;
const PADDING: u8 = 1;
const LINE_FEED: u8 = 2;
const WHITESPACE: u8 = 3;
const COMMENT: u8 = 4;

// Combine the alphabet, padding, line breaks and whitespace into one table so
// the inner loop needs just one lookup rather than repeated classification.
static CLASS: [u8; 256] = {
    let mut table = [5; 256];
    let mut i = 0;
    while i < 256 {
        let byte = i as u8;
        table[i] = match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => ALPHABET,
            b'=' => PADDING,
            b'\n' => LINE_FEED,
            b'\r' | b' ' | b'\t' => WHITESPACE,
            b'*' => COMMENT,
            _ => 5,
        };
        i += 1;
    }
    table
};

#[derive(Default)]
pub(super) struct Base64 {
    padding: u8,
    quantum: u8,
    invalid: bool,
    // Report each kind at most once per line: otherwise attacker-controlled
    // invalid octets could swamp the report with an issue per content byte.
    // Keep these latches across writes because one line can span many buffers.
    reported_invalid_character: bool,
    reported_comment: bool,
}

impl Base64 {
    pub(super) fn write(
        &mut self,
        bytes: &[u8],
        position: &mut Position,
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        if self.invalid {
            return Ok(());
        }
        for &byte in bytes {
            let category = CLASS[byte as usize];
            if self.padding == 0 {
                match category {
                    ALPHABET => self.quantum = (self.quantum + 1) & 3,
                    LINE_FEED => {
                        self.reported_invalid_character = false;
                        self.reported_comment = false;
                    }
                    WHITESPACE => {} // Whitespace is not encoded data, but is harmless.
                    PADDING => {
                        // '=' ends the content (except for further padding).
                        // Padding is valid only in the final quantum's last two positions.
                        if self.quantum < 2 {
                            position.log(logger, Violation::InvalidBase64Padding)?;
                            self.invalid = true;
                        } else {
                            self.padding = 1;
                            self.quantum = (self.quantum + 1) & 3;
                        }
                    }
                    COMMENT => {
                        // RFC 1113 allowed comments in the predecessor to Base64.
                        // RFC 1421 obsoleted it and MIME's RFC 1341 disallowed
                        // comments, but some mailers still generate them.
                        if !self.reported_comment {
                            self.reported_comment = true;
                            position.log(logger, Violation::ObsoleteBase64Comment)?;
                        }
                    }
                    _ => {
                        if !self.reported_invalid_character {
                            self.reported_invalid_character = true;
                            position.log(logger, Violation::InvalidBase64Character)?;
                        }
                    }
                }
            } else {
                match category {
                    LINE_FEED | WHITESPACE => {}
                    PADDING => {
                        self.quantum = (self.quantum + 1) & 3;
                        self.padding += 1;
                        if self.padding > 2 {
                            position.log(logger, Violation::InvalidBase64Padding)?;
                            self.invalid = true;
                        }
                    }
                    _ => {
                        position.log(logger, Violation::Base64CharactersAfterPadding)?;
                        self.invalid = true;
                    }
                }
            }
            position.advance(byte);
            if self.invalid {
                break;
            }
        }
        Ok(())
    }

    pub(super) fn finish(
        &self,
        position: &Position,
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        if !self.invalid && self.quantum != 0 {
            position.log(logger, Violation::IncompleteBase64Quantum)?;
        }
        Ok(())
    }
}

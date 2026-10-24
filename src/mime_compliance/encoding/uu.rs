// Adapted from MimeKit's Encodings/UUValidator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{MimeComplianceLogger, Position, Violation, io};

#[derive(Default, PartialEq)]
enum State {
    #[default]
    ExpectBegin,
    Begin(usize),
    FileMode,
    FileName,
    Payload,
    Ended,
    EndMarker(usize),
    End,
    Invalid,
}

pub(super) struct Uu {
    state: State,
    invalid_pretext: bool,
    invalid_file_mode: bool,
    // Latch each violation once per encoded line, including across writes.
    // Otherwise a malformed line could emit one finding per content byte.
    reported_invalid_content: bool,
    reported_extra_data: bool,
    eoln: bool,
    saved: u8,
    remaining: u8,
}

impl Default for Uu {
    fn default() -> Self {
        Self {
            state: State::ExpectBegin,
            invalid_pretext: false,
            invalid_file_mode: false,
            reported_invalid_content: false,
            reported_extra_data: false,
            eoln: false,
            // While finding "begin", saved holds the previous character.
            // Start at the beginning of a line.
            saved: b'\n',
            remaining: 0,
        }
    }
}

fn whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

impl Uu {
    fn pretext(
        &mut self,
        position: &Position,
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        if !self.invalid_pretext {
            position.log_at(
                logger,
                Violation::InvalidUUEncodePretext,
                position.offset - 1,
            )?;
            self.invalid_pretext = true;
        }
        self.state = State::ExpectBegin;
        Ok(())
    }

    fn skip_to_lf(bytes: &[u8], index: &mut usize, position: &mut Position) {
        let count = memchr::memchr(b'\n', &bytes[*index..]).unwrap_or(bytes.len() - *index);
        *index += count;
        position.offset += count as u64;
    }

    pub(super) fn write(
        &mut self,
        bytes: &[u8],
        position: &mut Position,
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        let mut index = 0;
        while index < bytes.len() {
            match self.state {
                State::ExpectBegin => {
                    if self.saved != b'\n' {
                        // Skip ahead to the next line.
                        Self::skip_to_lf(bytes, &mut index, position);
                        if index == bytes.len() {
                            self.saved = bytes[index - 1];
                            break;
                        }
                        self.saved = bytes[index];
                        position.advance(bytes[index]);
                        index += 1;
                        if index == bytes.len() {
                            break;
                        }
                    }
                    self.saved = bytes[index];
                    position.advance(bytes[index]);
                    index += 1;
                    if self.saved == b'b' {
                        self.state = State::Begin(1);
                    } else if !whitespace(self.saved) {
                        // Only whitespace lines are allowed before the begin marker.
                        self.pretext(position, logger)?;
                    }
                }
                State::Begin(matched) => {
                    self.saved = bytes[index];
                    position.advance(bytes[index]);
                    index += 1;
                    if self.saved != b"begin "[matched] {
                        self.pretext(position, logger)?;
                    } else if matched == 5 {
                        self.state = State::FileMode;
                        self.saved = 0;
                    } else {
                        self.state = State::Begin(matched + 1);
                    }
                }
                State::FileMode => {
                    // Saturate the digit count rather than letting a long mode
                    // wrap it. Report at the fifth digit as soon as it arrives,
                    // so the diagnostic does not depend on input chunking.
                    while index < bytes.len() && bytes[index].is_ascii_digit() {
                        if self.saved == 4 && !self.invalid_file_mode {
                            position.log(logger, Violation::InvalidUUEncodeFileMode)?;
                            self.invalid_file_mode = true;
                        }
                        self.saved = (self.saved + 1).min(5);
                        index += 1;
                        position.offset += 1;
                    }
                    if index == bytes.len() {
                        break;
                    }
                    if !self.invalid_file_mode && self.saved < 3 {
                        // The file mode is too short.
                        position.log(logger, Violation::InvalidUUEncodeFileMode)?;
                        self.invalid_file_mode = true;
                    }
                    // Scan ahead for the space after the mode.
                    while index < bytes.len() && bytes[index] != b' ' {
                        if !self.invalid_file_mode {
                            position.log(logger, Violation::InvalidUUEncodeFileMode)?;
                            self.invalid_file_mode = true;
                        }
                        if bytes[index] == b'\n' {
                            break;
                        }
                        index += 1;
                        position.offset += 1;
                    }
                    if index == bytes.len() {
                        break;
                    }
                    if bytes[index] != b'\n' {
                        index += 1;
                        position.offset += 1;
                    }
                    self.state = State::FileName;
                    self.saved = 0;
                }
                State::FileName => {
                    Self::skip_to_lf(bytes, &mut index, position);
                    if index == bytes.len() {
                        // Keep reading until the end of the filename line.
                        break;
                    }
                    position.advance(bytes[index]);
                    index += 1;
                    self.state = State::Payload;
                    self.eoln = true;
                    self.saved = 0;
                }
                State::Payload => {
                    // All bytes in 33..96 are payload characters; the length
                    // octet says exactly how many are needed, so consume runs.
                    // Once remaining reaches zero, the first extra byte must
                    // use the per-byte path to report data beyond the length.
                    if !self.eoln && (self.remaining > 0 || self.reported_extra_data) {
                        let input = &bytes[index..];
                        // After reporting invalid content, classification no
                        // longer affects quantum bookkeeping: scan to CR/LF.
                        // The same applies to extra data; classifying garbage
                        // beyond the payload would make reports depend on it.
                        let mut count = if self.reported_invalid_content || self.reported_extra_data
                        {
                            memchr::memchr2(b'\r', b'\n', input).unwrap_or(input.len())
                        } else {
                            input
                                .iter()
                                .position(|b| !(33..=96).contains(b))
                                .unwrap_or(input.len())
                        };
                        if self.remaining > 0 {
                            // Four characters encode three octets. Subtract
                            // characters already saved from a partial quantum.
                            let needed = 4 * usize::from(self.remaining).div_ceil(3)
                                - usize::from(self.saved);
                            count = count.min(needed);
                            if count > 0 {
                                let total = usize::from(self.saved) + count;
                                self.saved = (total % 4) as u8;
                                self.remaining =
                                    self.remaining.saturating_sub((3 * (total / 4)) as u8);
                                index += count;
                                position.offset += count as u64;
                                continue;
                            }
                        } else if count > 0 {
                            // Extra data was already reported; there is no
                            // quantum bookkeeping left to preserve.
                            index += count;
                            position.offset += count as u64;
                            continue;
                        }
                    }
                    let byte = bytes[index];
                    if byte == b'\r' {
                        position.advance(byte);
                        index += 1;
                        continue;
                    }
                    if byte == b'\n' {
                        if self.remaining > 0 {
                            position.log(logger, Violation::IncompleteUUEncodedLine)?;
                        }
                        position.advance(byte);
                        index += 1;
                        self.eoln = true;
                        continue;
                    }
                    if self.eoln {
                        // The first octet on a line declares its decoded length.
                        self.eoln = false;
                        self.reported_invalid_content = false;
                        self.reported_extra_data = false;
                        if byte == b'`' {
                            self.state = State::Ended;
                        } else {
                            self.remaining = byte.wrapping_sub(b' ') & 63;
                            if self.remaining > 45 {
                                position.log(logger, Violation::InvalidUUEncodedLineLength)?;
                            }
                        }
                        position.advance(byte);
                        index += 1;
                        continue;
                    }
                    position.advance(byte);
                    index += 1;
                    // Bytes beyond the declared length are not payload. Report
                    // them as extra data, not as a second content defect whose
                    // presence depends on what the trailing garbage looks like.
                    if self.remaining > 0 {
                        if !(33..=96).contains(&byte) && !self.reported_invalid_content {
                            self.reported_invalid_content = true;
                            position.log_at(
                                logger,
                                Violation::InvalidUUEncodedContent,
                                position.offset - 1,
                            )?;
                        }
                        self.saved += 1;
                        if self.saved == 4 {
                            self.remaining = self.remaining.saturating_sub(3);
                            self.saved = 0;
                        }
                    } else if !self.reported_extra_data {
                        self.reported_extra_data = true;
                        position.log_at(
                            logger,
                            Violation::InvalidUUEncodedLineExtraData,
                            position.offset - 1,
                        )?;
                    }
                }
                State::Ended => {
                    let byte = bytes[index];
                    if !whitespace(byte) {
                        position.log(logger, Violation::InvalidUUEncodeEndMarker)?;
                    }
                    position.advance(byte);
                    index += 1;
                    if byte == b'\n' {
                        self.state = State::EndMarker(0);
                    }
                }
                State::EndMarker(matched) => {
                    if bytes[index] != b"end"[matched] {
                        position.log(logger, Violation::InvalidUUEncodeEndMarker)?;
                        self.state = State::Invalid;
                        return Ok(());
                    }
                    position.advance(bytes[index]);
                    index += 1;
                    self.state = if matched == 2 {
                        State::End
                    } else {
                        State::EndMarker(matched + 1)
                    };
                }
                State::End => {
                    if !whitespace(bytes[index]) {
                        position.log(logger, Violation::InvalidUUEncodeEndMarker)?;
                        self.state = State::Invalid;
                        return Ok(());
                    }
                    position.advance(bytes[index]);
                    index += 1;
                }
                State::Invalid => return Ok(()),
            }
        }
        Ok(())
    }

    pub(super) fn finish(
        &self,
        position: &Position,
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        if !matches!(self.state, State::End | State::Invalid) {
            position.log(logger, Violation::IncompleteUUEncodedContent)?;
        }
        Ok(())
    }
}

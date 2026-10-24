// Adapted from MimeKit's Encodings/QuotedPrintableValidator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{MimeComplianceLogger, Position, Violation, io};

#[derive(Default, PartialEq)]
enum State {
    #[default]
    PassThrough,
    EqualSign,
    SoftBreak,
    DecodeByte,
}

#[derive(Default)]
pub(super) struct QuotedPrintable {
    state: State,
    // Report each kind at most once per line. "=~=~=~..." must not emit one
    // issue per two bytes and swamp other findings. Latches survive refills.
    reported_invalid_encoding: bool,
    reported_invalid_soft_break: bool,
}

impl QuotedPrintable {
    fn newline(&mut self) {
        self.reported_invalid_encoding = false;
        self.reported_invalid_soft_break = false;
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
                State::PassThrough => {
                    // Scan short runs scalarly: heavily encoded, non-Latin text
                    // has frequent '=' characters, making bulk-search setup costlier.
                    let end = bytes.len().min(index + 16);
                    while index < end {
                        let byte = bytes[index];
                        index += 1;
                        position.advance(byte);
                        if byte == b'=' {
                            self.state = State::EqualSign;
                            break;
                        }
                        if byte == b'\n' {
                            self.newline();
                        }
                    }
                    if self.state != State::PassThrough {
                        continue;
                    }
                    // Most quoted-printable bytes are literal characters.
                    // Vectorized search finds the next byte needing inspection.
                    while index < bytes.len() {
                        let Some(count) = memchr::memchr2(b'=', b'\n', &bytes[index..]) else {
                            position.offset += (bytes.len() - index) as u64;
                            return Ok(());
                        };
                        position.offset += count as u64;
                        index += count;
                        let byte = bytes[index];
                        index += 1;
                        position.advance(byte);
                        if byte == b'=' {
                            self.state = State::EqualSign;
                            break;
                        }
                        self.newline();
                    }
                }
                State::EqualSign => {
                    let byte = bytes[index];
                    self.state = match byte {
                        b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' => State::DecodeByte,
                        b'\r' => State::SoftBreak,
                        b'\n' => {
                            self.newline();
                            State::PassThrough
                        }
                        _ => {
                            if !self.reported_invalid_encoding {
                                self.reported_invalid_encoding = true;
                                position.log(logger, Violation::InvalidQuotedPrintableEncoding)?;
                            }
                            State::PassThrough
                        }
                    };
                    position.advance(byte);
                    index += 1;
                }
                State::SoftBreak => {
                    if bytes[index] == b'\n' {
                        position.advance(bytes[index]);
                        index += 1;
                        self.newline();
                    } else if !self.reported_invalid_soft_break {
                        self.reported_invalid_soft_break = true;
                        position.log(logger, Violation::InvalidQuotedPrintableSoftBreak)?;
                    }
                    self.state = State::PassThrough;
                }
                State::DecodeByte => {
                    let byte = bytes[index];
                    if !byte.is_ascii_hexdigit() {
                        if !self.reported_invalid_encoding {
                            self.reported_invalid_encoding = true;
                            position.log(logger, Violation::InvalidQuotedPrintableEncoding)?;
                        }
                        if byte == b'\n' {
                            self.newline();
                        }
                    }
                    self.state = State::PassThrough;
                    position.advance(byte);
                    index += 1;
                }
            }
        }
        Ok(())
    }

    pub(super) fn finish(
        &self,
        position: &Position,
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        // The only valid state to end on is pass-through.
        match self.state {
            State::EqualSign | State::DecodeByte => {
                position.log(logger, Violation::InvalidQuotedPrintableEncoding)
            }
            State::SoftBreak => position.log(logger, Violation::InvalidQuotedPrintableSoftBreak),
            State::PassThrough => Ok(()),
        }
    }
}

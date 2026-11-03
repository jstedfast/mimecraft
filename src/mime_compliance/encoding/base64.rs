// Adapted from MimeKit's Encodings/Base64Validator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{MimeComplianceLogger, Position, Violation, io};
use crate::encodings::Base64Decoder;

#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
#[path = "base64/x86.rs"]
mod simd;
#[cfg(all(feature = "simd", target_arch = "aarch64"))]
#[path = "base64/neon.rs"]
mod simd;
#[cfg(test)]
#[path = "base64/tests.rs"]
mod tests;

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
#[derive(Default)]
struct Prefix {
    len: usize,
    alphabet: usize,
    lines: u64,
    line_start: usize,
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
impl Prefix {
    // Only consume bytes preceding the first '=', '*' or invalid character.
    fn masks(&mut self, width: usize, mut alphabet: u32, mut lf: u32, valid: u32) -> bool {
        let count = ((!valid).trailing_zeros() as usize).min(width);
        if count < width {
            let mask = (1u32 << count) - 1;
            alphabet &= mask;
            lf &= mask;
        }
        // Whitespace is harmless but contributes nothing to the quantum.
        // Count only alphabet lanes and line feeds in the consumed prefix.
        self.alphabet += alphabet.count_ones() as usize;
        if lf != 0 {
            // For both vector widths the next line starts at 32 - leading_zeros.
            self.line_start = self.len + (32 - lf.leading_zeros()) as usize;
            self.lines += u64::from(lf.count_ones());
        }
        self.len += count;
        count == width
    }
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
struct Scanner(Option<simd::Accelerator>);

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
impl Default for Scanner {
    fn default() -> Self {
        Self(simd::Accelerator::detect())
    }
}

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
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    scanner: Scanner,
    padding: u8,
    quantum: u8,
    last_alphabet: u8,
    invalid: bool,
    // Report each kind at most once per line: otherwise attacker-controlled
    // invalid octets could swamp the report with an issue per content byte.
    // Keep these latches across writes because one line can span many buffers.
    reported_invalid_character: bool,
    reported_comment: bool,
}

impl Base64 {
    // Recover the last alphabet character only at chunk ends and padding,
    // rather than adding tracking work to every scalar byte and SIMD block.
    fn last_alphabet(&self, bytes: &[u8]) -> u8 {
        bytes
            .iter()
            .rev()
            .copied()
            .find(|&byte| CLASS[byte as usize] == ALPHABET)
            .unwrap_or(self.last_alphabet)
    }

    pub(super) fn write(
        &mut self,
        bytes: &[u8],
        position: &mut Position,
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        if self.invalid {
            return Ok(());
        }
        let mut index = 0;
        while index < bytes.len() {
            // Most content is alphabet characters and line breaks. Validate
            // these in bulk, then handle the first special byte scalarly.
            #[cfg(all(
                feature = "simd",
                any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
            ))]
            if self.padding == 0
                && bytes.len() - index >= 16
                && let Some(accelerator) = self.scanner.0
            {
                let prefix = accelerator.scan(&bytes[index..]);
                self.quantum = (self.quantum + (prefix.alphabet & 3) as u8) & 3;
                if prefix.lines != 0 {
                    position.line_start = position.offset + prefix.line_start as u64;
                    position.line += prefix.lines;
                    // A new line re-arms both once-per-line diagnostics, even
                    // when the preceding malformed line spanned writes.
                    self.reported_invalid_character = false;
                    self.reported_comment = false;
                }
                position.offset += prefix.len as u64;
                index += prefix.len;
                if index == bytes.len() {
                    break;
                }
            }
            let byte = bytes[index];
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
                            let mask = if self.quantum == 2 { 0x0f } else { 0x03 };
                            let last = self.last_alphabet(&bytes[..index]);
                            if Base64Decoder::rank(last) & mask != 0 {
                                position.log(logger, Violation::InvalidBase64Padding)?;
                            }
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
            index += 1;
            if self.invalid {
                break;
            }
        }
        // Only a partial quantum that can accept padding needs its last alphabet character.
        if self.quantum >= 2 && self.padding == 0 {
            self.last_alphabet = self.last_alphabet(bytes);
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

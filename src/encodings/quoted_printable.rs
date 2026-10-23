// Ported from MimeKit.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    EncoderOptionsError, InvalidLineLength, NewLineFormat,
    hex::{encode_hex_byte, hex_rank},
};

/// An incremental quoted-printable encoder for MIME bodies.
///
/// Uses MimeKit's line wrapping and trailing whitespace rules. CRLF and LF input
/// line breaks are normalized to the selected ending, which defaults to CRLF.
/// Use [`with_new_line_format`](Self::with_new_line_format) or
/// [`with_options`](Self::with_options) to select LF. Lone CR bytes become `=0D`.
///
/// Call [`finish`](Self::finish) to emit pending input. If the body does not
/// end in a newline, it ends with a soft break (`=` plus the selected ending)
/// so decoding does not introduce an extra newline. Dropping the encoder does
/// not flush it.
///
/// ```
/// use mimecraft::encodings::{QuotedPrintableDecoder, QuotedPrintableEncoder};
///
/// let mut encoded = Vec::new();
/// let mut encoder = QuotedPrintableEncoder::new();
/// encoder.encode(b"hello=world ", &mut encoded);
/// encoder.finish(&mut encoded);
/// assert_eq!(encoded, b"hello=3Dworld=20=\r\n");
///
/// let mut decoded = Vec::new();
/// QuotedPrintableDecoder::new().decode(&encoded, &mut decoded);
/// assert_eq!(decoded, b"hello=world ");
/// ```
#[derive(Clone, Debug)]
pub struct QuotedPrintableEncoder {
    line_length: usize,
    new_line_format: NewLineFormat,
    column: usize,
    // Delay one octet to determine whether a space or tab ends the line.
    pending: Option<u8>,
    // A CR may begin a CRLF pair split across input chunks.
    pending_cr: bool,
}

impl Default for QuotedPrintableEncoder {
    fn default() -> Self {
        Self {
            line_length: 76,
            new_line_format: NewLineFormat::default(),
            column: 0,
            pending: None,
            pending_cr: false,
        }
    }
}

impl QuotedPrintableEncoder {
    /// Creates an encoder with CRLF endings and a 76-character maximum line length.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an encoder with CRLF endings and a requested maximum line length.
    ///
    /// Accepted values are capped at 76, without rounding. Newline bytes are
    /// excluded from the limit; a soft break's `=` is included.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidLineLength`] if the value is outside 60..=998.
    pub fn with_line_length(max_line_length: usize) -> Result<Self, InvalidLineLength> {
        if !(60..=998).contains(&max_line_length) {
            return Err(InvalidLineLength {
                requested: max_line_length,
            });
        }

        // RFC 2045 requires quoted-printable lines to be at most 76 characters.
        Ok(Self {
            line_length: max_line_length.min(76),
            ..Self::default()
        })
    }

    /// Creates an encoder with the selected endings and a 76-character line length.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for
    /// [`NewLineFormat::Mixed`].
    pub fn with_new_line_format(
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        Self::with_options(76, new_line_format)
    }

    /// Creates an encoder with a requested line length and line ending format.
    ///
    /// The format applies to both hard and soft line breaks. Hard line breaks
    /// remain in the decoded content. The length is capped at 76 without rounding
    /// and excludes newline bytes; a soft break's `=` is included.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for
    /// [`NewLineFormat::Mixed`], or [`EncoderOptionsError::InvalidLineLength`]
    /// if the length is outside 60..=998. The format is checked first.
    ///
    /// ```
    /// use mimecraft::encodings::{NewLineFormat, QuotedPrintableEncoder};
    ///
    /// let mut encoder = QuotedPrintableEncoder::with_options(60, NewLineFormat::Unix).unwrap();
    /// let mut output = Vec::new();
    /// encoder.encode(b"hello\r\nworld ", &mut output);
    /// encoder.finish(&mut output);
    /// assert_eq!(output, b"hello\nworld=20=\n");
    /// ```
    pub fn with_options(
        max_line_length: usize,
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        if new_line_format == NewLineFormat::Mixed {
            return Err(EncoderOptionsError::InvalidNewLineFormat(new_line_format));
        }

        Ok(Self {
            new_line_format,
            ..Self::with_line_length(max_line_length)
                .map_err(EncoderOptionsError::InvalidLineLength)?
        })
    }

    /// Returns the effective maximum line length, excluding newline bytes.
    pub fn line_length(&self) -> usize {
        self.line_length
    }

    /// Returns the selected line ending format for hard and soft breaks.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    fn soft_break(&mut self, output: &mut Vec<u8>) {
        output.push(b'=');
        output.extend_from_slice(self.new_line_format.as_bytes());
        self.column = 0;
    }

    fn write_pending(&mut self, output: &mut Vec<u8>, end_of_line: bool, encode_blank: bool) {
        if let Some(byte) = self.pending.take() {
            let blank = matches!(byte, b' ' | b'\t');
            let safe = matches!(byte, b'\t' | b' '..=b'<' | b'>'..=b'~');
            let encode = !safe || (encode_blank && blank);
            let length = if encode { 3 } else { 1 };

            // An octet at the end of a line must fit within line_length; any
            // other octet must leave room for a soft break's '=' to follow it.
            if self.column + length + usize::from(!end_of_line) > self.line_length {
                self.soft_break(output);
            }

            if encode {
                encode_hex_byte(b'=', byte, output);
            } else {
                output.push(byte);
            }
            self.column += length;
        }
    }

    /// Appends encoded input, retaining an octet and possibly a following CR.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    // Spaces and tabs must be encoded if they end the line.
                    self.write_pending(output, true, true);
                    output.extend_from_slice(self.new_line_format.as_bytes());
                    self.column = 0;
                    continue;
                }

                // A lone CR is not a line break; encode it like any other octet.
                self.write_pending(output, false, false);
                self.pending = Some(b'\r');
            }

            match byte {
                b'\r' => self.pending_cr = true,
                b'\n' => {
                    // Spaces and tabs must be encoded if they end the line.
                    self.write_pending(output, true, true);
                    output.extend_from_slice(self.new_line_format.as_bytes());
                    self.column = 0;
                }
                _ => {
                    if self.pending.is_some() {
                        self.write_pending(output, false, false);
                        if self.column + 1 >= self.line_length {
                            self.soft_break(output);
                        }
                    }
                    self.pending = Some(byte);
                }
            }
        }
    }

    /// Emits pending input and a final soft break if the body needs one.
    ///
    /// Resets stream state while retaining the line length and ending. Empty
    /// bodies emit nothing; repeated calls without intervening input have no effect.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        if self.pending_cr {
            self.write_pending(output, false, false);
            self.pending = Some(b'\r');
        }
        if self.pending.is_some() {
            // Spaces and tabs must be encoded at the end of the content.
            // End with a soft line break so it isn't included in the decoded content.
            self.write_pending(output, false, true);
            self.soft_break(output);
        }
        self.reset();
    }

    /// Discards pending input, retaining the line length and ending settings.
    ///
    /// Use [`finish`](Self::finish) instead to complete the current body.
    pub fn reset(&mut self) {
        self.column = 0;
        self.pending = None;
        self.pending_cr = false;
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum DecoderState {
    #[default]
    PassThrough,
    EqualSign,
    SoftBreak,
    // Keep the original digit too, so malformed sequences pass through unchanged.
    HexDigit {
        byte: u8,
        rank: u8,
    },
}

/// An incremental, tolerant quoted-printable decoder.
///
/// Accepts upper- and lowercase hex digits and removes both `=\n` and `=\r\n`
/// soft breaks. Other line endings and whitespace pass through unchanged.
/// Malformed escapes pass through literally, matching MimeKit.
///
/// Incomplete escapes (`=`, `=X` with a hex digit, or `=\r`) remain pending
/// until more input arrives or [`reset`](Self::reset) discards them. There is
/// no implicit flushing or replacement of incomplete escapes.
#[derive(Clone, Debug, Default)]
pub struct QuotedPrintableDecoder {
    state: DecoderState,
    rfc2047: bool,
}

impl QuotedPrintableDecoder {
    /// Creates a decoder for MIME bodies, where underscores remain literal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a decoder for the payload of an RFC 2047 Q-encoded word.
    ///
    /// Underscores in pass-through text become spaces; malformed escapes remain
    /// unchanged. This does not parse the encoded-word wrapper, convert character
    /// sets, or join adjacent encoded words.
    pub fn for_encoded_words() -> Self {
        Self {
            rfc2047: true,
            ..Self::default()
        }
    }

    /// Appends decoded input to `output`, retaining incomplete escapes.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn decode(&mut self, mut input: &[u8], output: &mut Vec<u8>) {
        while let Some((&byte, rest)) = input.split_first() {
            match self.state {
                DecoderState::PassThrough => {
                    // Scan the first few bytes individually: heavily encoded
                    // content has short runs between '=' characters, where
                    // setting up a separate search and copy is not worthwhile.
                    // Q-encoded words also require per-byte underscore handling.
                    let scan_length = if self.rfc2047 {
                        input.len()
                    } else {
                        input.len().min(16)
                    };
                    let mut consumed = 0;
                    for &byte in &input[..scan_length] {
                        consumed += 1;
                        if byte == b'=' {
                            self.state = DecoderState::EqualSign;
                            break;
                        }
                        output.push(if self.rfc2047 && byte == b'_' {
                            b' '
                        } else {
                            byte
                        });
                    }
                    input = &input[consumed..];

                    if !self.rfc2047 && matches!(self.state, DecoderState::PassThrough) {
                        let count = input.iter().position(|&b| b == b'=').unwrap_or(input.len());
                        output.extend_from_slice(&input[..count]);
                        input = &input[count..];
                        if !input.is_empty() {
                            self.state = DecoderState::EqualSign;
                            input = &input[1..];
                        }
                    }
                }
                DecoderState::EqualSign => {
                    input = rest;
                    if let Some(rank) = hex_rank(byte) {
                        self.state = DecoderState::HexDigit { byte, rank };
                    } else {
                        match byte {
                            // Invalid sequence: emit the first '=' but keep the
                            // second as a potential escape or soft-break prefix.
                            b'=' => output.push(b'='),
                            b'\r' => self.state = DecoderState::SoftBreak,
                            b'\n' => self.state = DecoderState::PassThrough,
                            _ => {
                                // Invalid encoded sequence: pass it through undecoded.
                                output.extend_from_slice(&[b'=', byte]);
                                self.state = DecoderState::PassThrough;
                            }
                        }
                    }
                }
                DecoderState::SoftBreak => {
                    input = rest;
                    if byte != b'\n' {
                        // Invalid encoded sequence: pass it through undecoded.
                        output.extend_from_slice(&[b'=', b'\r', byte]);
                    }
                    self.state = DecoderState::PassThrough;
                }
                DecoderState::HexDigit { byte: saved, rank } => {
                    input = rest;
                    if let Some(low) = hex_rank(byte) {
                        output.push((rank << 4) | low);
                    } else {
                        // Invalid encoded sequence: preserve the original digit's
                        // case and consume the bad byte without reinterpreting it.
                        output.extend_from_slice(&[b'=', saved, byte]);
                    }
                    self.state = DecoderState::PassThrough;
                }
            }
        }
    }

    /// Discards an incomplete escape, retaining body or encoded-word mode.
    pub fn reset(&mut self) {
        self.state = DecoderState::PassThrough;
    }
}

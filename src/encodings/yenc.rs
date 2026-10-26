// Ported from MimeKit's YEncoder, YDecoder, and Crc32.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{EncoderOptionsError, InvalidLineLength, NewLineFormat};

// Generate the same reflected CRC-32 table as MimeKit at compile time. Both
// codecs retain the uncomplemented running state, initialized to all ones.
static CRC32_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    while index < table.len() {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};

fn update_crc(crc: u32, byte: u8) -> u32 {
    CRC32_TABLE[((crc ^ u32::from(byte)) & 255) as usize] ^ (crc >> 8)
}

/// An incremental yEnc payload encoder with CRC-32 tracking.
///
/// Defaults to 128 encoded octets per line and [`NewLineFormat::Dos`] (CRLF).
/// Unix (LF) endings and line lengths of 60..=998 are also supported. Input
/// bytes are encoded unchanged; no `=ybegin`, `=ypart`, or `=yend` wrapper is
/// written.
///
/// Each byte is offset by 42, wrapping at 256. NUL, tab, CR, LF, `=`, and `.`
/// in the resulting alphabet are escaped. An escape pair is never split:
/// matching MimeKit, a line may exceed its requested length by one octet.
///
/// Call [`finish`](Self::finish) to terminate an unfinished line. Unlike the
/// other encoders, this retains the running checksum; call [`reset`](Self::reset)
/// before encoding an independent body. Dropping the encoder does not flush it.
///
/// ```
/// use mimecraft::encodings::{YDecoder, YEncoder};
///
/// let input = [214, 224, 227, 19, 4, 223, 23];
/// let mut encoded = Vec::new();
/// let mut encoder = YEncoder::new();
/// encoder.encode(&input, &mut encoded);
/// encoder.finish(&mut encoded);
/// assert_eq!(encoded, b"=@=J=M=}=n=IA\r\n");
///
/// let mut decoded = Vec::new();
/// let mut decoder = YDecoder::for_payload();
/// decoder.decode(&encoded, &mut decoded);
/// assert_eq!(decoded, input);
/// assert_eq!(decoder.checksum(), encoder.checksum());
/// ```
#[derive(Clone, Debug)]
pub struct YEncoder {
    line_length: usize,
    new_line_format: NewLineFormat,
    column: usize,
    crc: u32,
}

impl Default for YEncoder {
    fn default() -> Self {
        Self {
            line_length: 128,
            new_line_format: NewLineFormat::default(),
            column: 0,
            crc: u32::MAX,
        }
    }
}

impl YEncoder {
    /// Creates an encoder with CRLF endings and a 128-octet line length.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an encoder with CRLF endings and the requested line length.
    ///
    /// The length is not capped or rounded. Newline bytes are excluded, and an
    /// escape pair may extend a line one octet beyond the requested length.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidLineLength`] if the length is outside 60..=998.
    pub fn with_line_length(max_line_length: usize) -> Result<Self, InvalidLineLength> {
        if !(60..=998).contains(&max_line_length) {
            return Err(InvalidLineLength {
                requested: max_line_length,
            });
        }
        Ok(Self {
            line_length: max_line_length,
            ..Self::default()
        })
    }

    /// Creates an encoder with the selected endings and a 128-octet line length.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for
    /// [`NewLineFormat::Mixed`].
    pub fn with_new_line_format(
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        Self::with_options(128, new_line_format)
    }

    /// Creates an encoder with the requested line length and new-line format.
    ///
    /// The length is not capped or rounded, excludes newline bytes, and may be
    /// exceeded by one octet to keep an escape pair together.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for
    /// [`NewLineFormat::Mixed`], or [`EncoderOptionsError::InvalidLineLength`]
    /// if the length is outside 60..=998. The format is checked first.
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

    /// Returns the requested encoded line length, excluding newline bytes.
    pub fn line_length(&self) -> usize {
        self.line_length
    }

    /// Returns the selected new-line format.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Returns MimeKit's uncomplemented running CRC-32 state as an unsigned value.
    ///
    /// Initially `u32::MAX`. Use `!encoder.checksum()` for the finalized CRC-32
    /// value written in a yEnc trailer. Only input bytes affect the checksum;
    /// line endings and escape bytes do not. [`finish`](Self::finish) retains it.
    pub fn checksum(&self) -> u32 {
        self.crc
    }

    /// Appends encoded bytes, preserving the line position and checksum.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            self.crc = update_crc(self.crc, byte);
            let encoded = byte.wrapping_add(42);
            if matches!(encoded, 0 | b'\t' | b'\r' | b'\n' | b'=' | b'.') {
                output.extend_from_slice(&[b'=', encoded.wrapping_add(64)]);
                self.column += 2;
            } else {
                output.push(encoded);
                self.column += 1;
            }
            // Wrap after writing the whole escape pair, even if it pushes the
            // line one octet past its configured length, matching MimeKit.
            if self.column >= self.line_length {
                output.extend_from_slice(self.new_line_format.as_bytes());
                self.column = 0;
            }
        }
    }

    /// Terminates a partial line, retaining the checksum and configuration.
    ///
    /// Empty input and already terminated lines produce no extra newline.
    /// Repeated calls have no effect. Subsequent encoding continues the checksum
    /// on a new line; call [`reset`](Self::reset) to begin an independent body.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        if self.column != 0 {
            output.extend_from_slice(self.new_line_format.as_bytes());
            self.column = 0;
        }
    }

    /// Discards the line position and checksum, retaining the configuration.
    pub fn reset(&mut self) {
        self.column = 0;
        self.crc = u32::MAX;
    }
}

#[derive(Clone, Copy, Debug)]
enum DecoderState {
    SeekingBegin,
    Begin { matched: usize },
    BeginHeader,
    PartOrPayload,
    Part { matched: usize },
    PartHeader,
    Payload,
    Ended,
}

/// An incremental, tolerant yEnc decoder with CRC-32 tracking.
///
/// By default, scans for a case-sensitive `=ybegin ` marker at a line start,
/// skips its header and an optional `=ypart ` header, then decodes payload.
/// Use [`for_payload`](Self::for_payload) for unwrapped encoder output.
/// Header fields, sizes, filenames, and trailer checksums are not validated
/// or returned, and multipart bodies are not assembled automatically.
///
/// Matches MimeKit's permissive behavior: CR/LF are discarded and cancel a
/// pending escape, LF starts a line, and `=y` at a payload line start ends the
/// stream without requiring the rest of `=yend`. Subsequent input is ignored
/// until [`reset`](Self::reset). A trailing `=` waits for more input.
///
/// # Compatibility
///
/// The optional-part scanner preserves a leading escape when the first line
/// after `=ybegin` is payload rather than an `=ypart` header.
/// Malformed `=y...` part candidates are skipped through LF before trying the
/// next line, while `=ye` ends the stream immediately.
#[derive(Clone, Debug)]
pub struct YDecoder {
    initial: DecoderState,
    state: DecoderState,
    escaped: bool,
    end_of_line: bool,
    crc: u32,
}

impl Default for YDecoder {
    fn default() -> Self {
        // Start at a line boundary so a marker at offset zero is accepted.
        let initial = DecoderState::Begin { matched: 0 };
        Self {
            initial,
            state: initial,
            escaped: false,
            end_of_line: true,
            crc: u32::MAX,
        }
    }
}

impl YDecoder {
    /// Creates a decoder that first searches for a `=ybegin ` line.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a decoder for payload without a `=ybegin` or `=ypart` header.
    pub fn for_payload() -> Self {
        Self {
            initial: DecoderState::Payload,
            state: DecoderState::Payload,
            ..Self::default()
        }
    }

    /// Returns MimeKit's uncomplemented running CRC-32 state as an unsigned value.
    ///
    /// Initially `u32::MAX`. Use `!decoder.checksum()` for the finalized CRC-32
    /// of decoded bytes. This is computed, not checked against any trailer.
    pub fn checksum(&self) -> u32 {
        self.crc
    }

    /// Appends decoded bytes, preserving partial markers, escapes, and checksum.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            match self.state {
                DecoderState::SeekingBegin => {
                    if byte == b'\n' {
                        self.state = DecoderState::Begin { matched: 0 };
                    }
                    continue;
                }
                DecoderState::Begin { matched } => {
                    if byte == b"=ybegin "[matched] {
                        self.state = if matched + 1 == b"=ybegin ".len() {
                            DecoderState::BeginHeader
                        } else {
                            DecoderState::Begin {
                                matched: matched + 1,
                            }
                        };
                    } else {
                        // A failed prefix consumes the rest of its line.
                        // Only LF, not CR or NUL, allows another marker.
                        self.state = if byte == b'\n' {
                            DecoderState::Begin { matched: 0 }
                        } else {
                            DecoderState::SeekingBegin
                        };
                    }
                    continue;
                }
                DecoderState::BeginHeader => {
                    if byte == b'\n' {
                        self.state = DecoderState::PartOrPayload;
                    }
                    continue;
                }
                DecoderState::PartOrPayload => {
                    if byte == b'=' {
                        self.state = DecoderState::Part { matched: 1 };
                        continue;
                    }
                    self.state = DecoderState::Payload;
                }
                DecoderState::Part { matched } => {
                    if matched == 1 && byte != b'y' {
                        // The candidate '=' belongs to the payload's first escape.
                        self.state = DecoderState::Payload;
                        self.escaped = true;
                    } else if matched == 2 && byte == b'e' {
                        // MimeKit treats "=ye" here as an "=yend" marker.
                        self.state = DecoderState::Ended;
                        return;
                    } else {
                        self.state = if byte == b"=ypart "[matched] {
                            if matched + 1 == b"=ypart ".len() {
                                DecoderState::PartHeader
                            } else {
                                DecoderState::Part {
                                    matched: matched + 1,
                                }
                            }
                        } else if byte == b'\n' {
                            DecoderState::PartOrPayload
                        } else {
                            DecoderState::BeginHeader
                        };
                        continue;
                    }
                }
                DecoderState::PartHeader => {
                    if byte == b'\n' {
                        self.state = DecoderState::Payload;
                    }
                    continue;
                }
                DecoderState::Payload => {}
                DecoderState::Ended => return,
            }

            if byte == b'\r' {
                self.escaped = false;
                continue;
            }
            if byte == b'\n' {
                self.escaped = false;
                self.end_of_line = true;
                continue;
            }

            let mut decoded = byte;
            if self.escaped {
                if self.end_of_line && byte == b'y' {
                    // This can only be "=yend" in canonical payload. As in
                    // MimeKit, stop at "=y" without validating the whole marker.
                    self.state = DecoderState::Ended;
                    return;
                }
                self.escaped = false;
                decoded = decoded.wrapping_sub(64);
            } else if byte == b'=' {
                self.escaped = true;
                continue;
            }
            self.end_of_line = false;
            decoded = decoded.wrapping_sub(42);
            self.crc = update_crc(self.crc, decoded);
            output.push(decoded);
        }
    }

    /// Resets all stream state and checksum, retaining the original scanning mode.
    pub fn reset(&mut self) {
        self.state = self.initial;
        self.escaped = false;
        self.end_of_line = true;
        self.crc = u32::MAX;
    }
}

// Ported from MimeKit's Tnef/RtfCompressedToRtf.cs and RtfCompressionMode.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::{crc32, filters::MimeFilter};

const INITIALIZER: &[u8; 207] = b"{\\rtf1\\ansi\\mac\\deff0\\deftab720{\\fonttbl;}{\\f0\\fnil \\froman \\fswiss \\fmodern \\fscript \\fdecor MS Sans SerifSymbolArialTimes New RomanCourier{\\colortbl\\red0\\green0\\blue0\r\n\\par \\pard\\plain\\f0\\fs20\\b\\i\\u\\tab\\tx";

/// The COMPTYPE field of an [MS-OXRTFCP] compressed RTF stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RtfCompressionMode {
    /// An unrecognized raw value; zero also represents an unread COMPTYPE field.
    Unknown(u32),
    /// Uncompressed RTF (MELA).
    Uncompressed,
    /// Dictionary-compressed RTF (LZFu).
    Compressed,
}

impl From<u32> for RtfCompressionMode {
    fn from(value: u32) -> Self {
        match value {
            0x414c454d => Self::Uncompressed,
            0x75465a4c => Self::Compressed,
            value => Self::Unknown(value),
        }
    }
}

impl From<RtfCompressionMode> for u32 {
    fn from(value: RtfCompressionMode) -> Self {
        match value {
            RtfCompressionMode::Uncompressed => 0x414c454d,
            RtfCompressionMode::Compressed => 0x75465a4c,
            RtfCompressionMode::Unknown(value) => value,
        }
    }
}

/// A framing or checksum failure reported by [`RtfCompressedToRtf::validate`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtfDecompressionError {
    /// Fewer than sixteen header bytes have been supplied.
    IncompleteHeader,
    /// COMPSIZE cannot contain the twelve remaining header bytes.
    InvalidCompressedSize(i32),
    /// The contents cannot be interpreted as compressed or uncompressed RTF.
    UnsupportedCompressionMode(u32),
    /// Neither the CONTENTS boundary nor a compressed end marker was reached.
    IncompleteContent,
    /// The checksum does not match; uncompressed streams require zero.
    InvalidCrc32 {
        /// CRC stored in the header.
        expected: u32,
        /// Computed CRC, or zero for an uncompressed stream.
        actual: u32,
    },
}

impl fmt::Display for RtfDecompressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompleteHeader => f.write_str("incomplete compressed RTF header"),
            Self::InvalidCompressedSize(size) => write!(f, "invalid compressed RTF size: {size}"),
            Self::UnsupportedCompressionMode(mode) => {
                write!(f, "unsupported RTF compression mode: {mode:#010x}")
            }
            Self::IncompleteContent => f.write_str("incomplete compressed RTF content"),
            Self::InvalidCrc32 { expected, actual } => write!(
                f,
                "invalid RTF CRC-32: expected {expected:#010x}, got {actual:#010x}"
            ),
        }
    }
}

impl Error for RtfDecompressionError {}

#[derive(Clone, Copy, Debug)]
enum State {
    Flags,
    Literal,
    ReferenceHigh,
    ReferenceLow(u8),
    Complete,
}

/// Incrementally decompresses an [MS-OXRTFCP] RTF stream without I/O.
///
/// Implements [`MimeFilter`] and works with [`crate::io::FilteredReader`] and
/// [`crate::io::FilteredWriter`]. Output is appended to the caller's buffer;
/// existing bytes are untouched and the buffer may be drained between calls.
/// Cloning snapshots the complete filter, including partial headers/tokens.
///
/// Decoding follows MimeKit's tolerant behavior: unknown compression modes
/// discard content, incomplete tokens emit nothing, RAWSIZE is not enforced,
/// and bytes beyond COMPSIZE or an end marker are discarded, not checksummed.
/// Output is provisional until [`validate`](Self::validate) succeeds. The
/// filter trait cannot return errors, so callers must explicitly validate;
/// the I/O adapters do not translate these failures into I/O errors.
///
/// The dictionary is a fixed inline 4 KiB array. Untrusted size fields never
/// drive allocation; output grows only when actual tokens produce bytes.
/// This is not RTF syntax validation, text rendering, or HTML extraction.
///
/// ```
/// use mimecraft::tnef::RtfCompressedToRtf;
/// # let input = b"\x18\0\0\0\x0c\0\0\0MELA\0\0\0\0{\\rtf1 test}";
/// let mut filter = RtfCompressedToRtf::new();
/// let mut output = Vec::new();
/// for chunk in input.chunks(3) {
///     filter.filter(chunk, &mut output);
/// }
/// filter.finish(&mut output);
/// filter.validate()?;
/// assert_eq!(output, b"{\\rtf1 test}");
/// # Ok::<(), mimecraft::tnef::RtfDecompressionError>(())
/// ```
#[derive(Clone, Debug)]
pub struct RtfCompressedToRtf {
    dictionary: [u8; 4096],
    write_offset: usize,
    header: [u8; 16],
    header_length: usize,
    mode: RtfCompressionMode,
    compressed_size: i32,
    content_length: usize,
    size: usize,
    expected_crc: u32,
    crc: u32,
    state: State,
    flags: u8,
    remaining_flags: u8,
}

impl Default for RtfCompressedToRtf {
    fn default() -> Self {
        Self::new()
    }
}

impl RtfCompressedToRtf {
    /// Creates a filter with the 207-byte dictionary initializer and zero CRC.
    pub fn new() -> Self {
        let mut dictionary = [0; 4096];
        dictionary[..INITIALIZER.len()].copy_from_slice(INITIALIZER);
        Self {
            dictionary,
            write_offset: INITIALIZER.len(),
            header: [0; 16],
            header_length: 0,
            mode: RtfCompressionMode::Unknown(0),
            compressed_size: 0,
            content_length: 0,
            size: 0,
            expected_crc: 0,
            crc: 0,
            state: State::Flags,
            flags: 0,
            remaining_flags: 0,
        }
    }

    /// The raw COMPTYPE interpretation, available after twelve input bytes.
    /// Before that, and after reset, returns `Unknown(0)`.
    pub fn compression_mode(&self) -> RtfCompressionMode {
        self.mode
    }

    /// Whether a recognized stream has reached its CONTENTS boundary or end
    /// marker. This does not imply a valid checksum or well-formed RTF syntax.
    pub fn is_complete(&self) -> bool {
        self.header_length == 16
            && self.compressed_size >= 12
            && !matches!(self.mode, RtfCompressionMode::Unknown(_))
            && (self.size == self.content_length || matches!(self.state, State::Complete))
    }

    /// Whether a recognized, complete stream has the required CRC.
    ///
    /// Unlike MimeKit's running equality check, this remains false before
    /// completion even when a partial checksum happens to match the header.
    pub fn is_valid_crc32(&self) -> bool {
        self.is_complete() && self.crc == self.expected_crc
    }

    /// Checks framing, recognized compression, completion, and the CRC without
    /// mutating state or discarding output. Call after supplying all input.
    ///
    /// Matches MimeKit's CONTENTS/end-marker boundaries, not strict RTF validity:
    /// RAWSIZE is not checked, nor is an end marker required at the size boundary.
    /// A partial token at that boundary is discarded as in the tolerant filter.
    ///
    /// # Errors
    ///
    /// Returns explicit header, size, compression-mode, truncation, or CRC errors.
    pub fn validate(&self) -> Result<(), RtfDecompressionError> {
        if self.header_length < 16 {
            return Err(RtfDecompressionError::IncompleteHeader);
        }
        if let RtfCompressionMode::Unknown(mode) = self.mode {
            return Err(RtfDecompressionError::UnsupportedCompressionMode(mode));
        }
        if self.compressed_size < 12 {
            return Err(RtfDecompressionError::InvalidCompressedSize(
                self.compressed_size,
            ));
        }
        if !self.is_complete() {
            return Err(RtfDecompressionError::IncompleteContent);
        }
        if self.crc != self.expected_crc {
            return Err(RtfDecompressionError::InvalidCrc32 {
                expected: self.expected_crc,
                actual: self.crc,
            });
        }
        Ok(())
    }

    fn next_token(&mut self) {
        self.remaining_flags -= 1;
        self.flags >>= 1;
        self.state = if self.remaining_flags == 0 {
            State::Flags
        } else if self.flags & 1 == 0 {
            State::Literal
        } else {
            State::ReferenceHigh
        };
    }

    fn append(&mut self, value: u8, output: &mut Vec<u8>) {
        output.push(value);
        self.dictionary[self.write_offset] = value;
        self.write_offset = (self.write_offset + 1) & 4095;
    }

    /// Processes input, retaining split headers and reference tokens.
    ///
    /// Subsequent calls after completion discard input until reset.
    pub fn filter(&mut self, mut input: &[u8], output: &mut Vec<u8>) {
        if self.header_length < 16 {
            let old_length = self.header_length;
            let count = input.len().min(16 - old_length);
            self.header[old_length..old_length + count].copy_from_slice(&input[..count]);
            self.header_length += count;
            input = &input[count..];
            if old_length < 12 && self.header_length >= 12 {
                self.mode = u32::from_le_bytes(self.header[8..12].try_into().unwrap()).into();
            }
            if self.header_length < 16 {
                return;
            }
            self.compressed_size = i32::from_le_bytes(self.header[..4].try_into().unwrap());
            // COMPSIZE is CONTENTS length plus the twelve remaining header
            // bytes. Smaller (including negative) values are nonsensical:
            // treat their CONTENTS as empty rather than allowing underflow.
            self.content_length = if self.compressed_size >= 12 {
                (self.compressed_size - 12) as usize
            } else {
                0
            };
            self.expected_crc = u32::from_le_bytes(self.header[12..16].try_into().unwrap());
        }

        match self.mode {
            RtfCompressionMode::Unknown(_) => {
                // [MS-OXRTFCP] 2.1.3.1.1 defines only COMPRESSED and UNCOMPRESSED.
                // Passing unknown content through would label arbitrary bytes
                // as RTF, so discard it; validate() reports the unsupported mode.
                return;
            }
            RtfCompressionMode::Uncompressed => {
                // UNCOMPRESSED requires a zero CRC; only COMPRESSED contents
                // have a computed checksum, so do not accumulate one here.
                let count = input.len().min(self.content_length - self.size);
                output.extend_from_slice(&input[..count]);
                self.size += count;
                return;
            }
            RtfCompressionMode::Compressed => {}
        }

        // COMPSIZE and RAWSIZE are untrusted. Unlike an estimated output-buffer
        // reservation, growing only for decoded tokens cannot overflow an
        // estimate or force a huge allocation from a bogus header.
        // Stop at CONTENTS length: following data must not enter output or CRC.
        for &value in input.iter().take(self.content_length - self.size) {
            if matches!(self.state, State::Complete) {
                break;
            }
            self.crc = crc32::update(self.crc, value);
            self.size += 1;
            match self.state {
                State::Flags => {
                    self.flags = value;
                    self.remaining_flags = 8;
                    self.state = if value & 1 == 0 {
                        State::Literal
                    } else {
                        State::ReferenceHigh
                    };
                }
                State::Literal => {
                    self.append(value, output);
                    self.next_token();
                }
                State::ReferenceHigh => self.state = State::ReferenceLow(value),
                State::ReferenceLow(high) => {
                    // A reference packs a 12-bit dictionary offset followed
                    // by a 4-bit length minus two, for runs of 2..=17 bytes.
                    let offset = (usize::from(high) << 4) | usize::from(value >> 4);
                    let length = usize::from(value & 15) + 2;
                    if offset == self.write_offset {
                        self.state = State::Complete;
                        continue;
                    }
                    // Copy byte by byte and write back as we go: references
                    // may overlap the bytes they produce and wrap at 4096.
                    for index in 0..length {
                        self.append(self.dictionary[(offset + index) & 4095], output);
                    }
                    self.next_token();
                }
                State::Complete => unreachable!(),
            }
        }
    }

    /// Emits nothing, retaining incomplete state as MimeKit's flush does.
    /// Repeated calls are harmless; use [`validate`](Self::validate) to detect
    /// incomplete input. Further input may resume decoding until completion.
    pub fn finish(&mut self, _output: &mut Vec<u8>) {}

    /// Discards stream state without output or allocation.
    pub fn reset(&mut self) {
        // Clear the entire dictionary, not just its initializer, so malformed
        // references cannot expose bytes retained from a previous stream.
        *self = Self::new();
    }
}

impl MimeFilter for RtfCompressedToRtf {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        Self::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        Self::finish(self, output);
    }

    fn reset(&mut self) {
        Self::reset(self);
    }
}

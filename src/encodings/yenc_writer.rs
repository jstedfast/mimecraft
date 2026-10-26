// Adapted from MimeKit's Encodings/YEncWriter.cs and YEncOptions.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Read, Write},
};

use crate::charsets::{Charset, CharsetError};

use super::{
    EncoderOptionsError, NewLineFormat, YEncHeader, YEncoder, yenc_header::MAX_YENC_LINE_LENGTH,
};

/// Options copied by [`YEncWriter::with_options`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct YEncWriterOptions {
    /// Maximum control/payload line length in bytes, excluding the newline.
    /// Defaults to 16384; valid values are 1 through `i32::MAX - 2`.
    pub max_line_length: usize,
    /// Maximum decoded size of each block, not the entire multipart file.
    /// Defaults to 1 GiB. Zero permits only empty single-part blocks.
    pub max_decoded_size: u64,
    /// Filename charset; defaults to exact Latin-1. Unmappable text is an error.
    /// yEnc does not declare a charset; readers must select the same encoding.
    pub file_name_charset: Charset,
    /// Output line endings. Defaults to CRLF; `Mixed` is rejected.
    pub new_line_format: NewLineFormat,
}

impl Default for YEncWriterOptions {
    fn default() -> Self {
        Self {
            max_line_length: 16 * 1024,
            max_decoded_size: 1024 * 1024 * 1024,
            file_name_charset: Charset::LATIN1,
            new_line_format: NewLineFormat::Dos,
        }
    }
}

/// A streaming writer for complete single-part and multipart yEnc blocks.
///
/// Writes framing, encoded payload, and finalized CRC-32 trailers without
/// buffering the entire file. Each source must end at exactly the declared
/// block size, starting at its current position. Multipart sources contain only
/// the indicated range; the writer neither seeks nor assembles parts.
///
/// Preflight errors perform no I/O and leave the writer usable. Once I/O starts,
/// any failure permanently faults the writer and may leave incomplete output.
/// Sources are borrowed; use a borrowed destination to retain ownership of it.
/// Neither writing nor dropping flushes the destination. This is not a
/// [`Write`] adapter and does not generate MIME headers or NNTP framing.
///
/// ```
/// use mimecraft::encodings::{YEncHeader, YEncWriter};
///
/// let mut writer = YEncWriter::new(Vec::new());
/// let header = YEncHeader::new("test.bin", 9)?;
/// let crc = writer.write_block(&header, &mut &b"123456789"[..], None)?;
/// assert_eq!(crc, 0xcbf43926);
/// assert!(writer.into_inner().ends_with(b"=yend size=9 crc32=cbf43926\r\n"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct YEncWriter<W> {
    inner: W,
    options: YEncWriterOptions,
    faulted: bool,
}

impl<W: Write> YEncWriter<W> {
    /// Wraps a destination with default options without performing I/O.
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            options: YEncWriterOptions::default(),
            faulted: false,
        }
    }

    /// Wraps a destination and copies options without performing I/O.
    ///
    /// # Errors
    ///
    /// Rejects an invalid maximum line length or mixed output line endings.
    pub fn with_options(inner: W, options: &YEncWriterOptions) -> Result<Self, YEncWriteError> {
        if !(1..=MAX_YENC_LINE_LENGTH).contains(&options.max_line_length) {
            return Err(YEncWriteError::InvalidMaxLineLength(
                options.max_line_length,
            ));
        }
        if options.new_line_format == NewLineFormat::Mixed {
            return Err(YEncWriteError::Encoder(
                EncoderOptionsError::InvalidNewLineFormat(options.new_line_format),
            ));
        }
        Ok(Self {
            inner,
            options: *options,
            faulted: false,
        })
    }

    /// Returns the underlying destination.
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Returns mutable access to the destination, bypassing block framing.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    /// Returns the destination without flushing or writing anything.
    pub fn into_inner(self) -> W {
        self.inner
    }

    /// Returns whether a previous block failed after I/O began.
    pub fn is_faulted(&self) -> bool {
        self.faulted
    }

    /// Writes one block and returns its finalized CRC-32.
    ///
    /// A supplied whole-file checksum is verified for single-part blocks and
    /// multipart ranges covering the entire file. For partial ranges it is
    /// emitted without verification. A one-byte excess-source probe can consume
    /// one byte beyond the declared block size.
    ///
    /// # Errors
    ///
    /// Preflight rejects limits, unsupported payload line lengths (outside
    /// 60..=998), unencodable filenames, and encoded filenames containing NUL,
    /// CR, or LF. I/O failures, source size mismatches, and checksum mismatches
    /// fault the writer. Interrupted I/O is retried; all other errors propagate.
    /// No further blocks may be written after a fault.
    pub fn write_block<R: Read + ?Sized>(
        &mut self,
        header: &YEncHeader,
        source: &mut R,
        file_checksum: Option<u32>,
    ) -> Result<u32, YEncWriteError> {
        if self.faulted {
            return Err(YEncWriteError::Faulted);
        }
        let encoder = YEncoder::with_options(header.line_length(), self.options.new_line_format)
            .map_err(YEncWriteError::Encoder)?;
        let begin = self.create_header(header, file_checksum)?;

        self.faulted = true;
        let result = self.write_inner(header, source, file_checksum, &begin, encoder);
        if result.is_ok() {
            self.faulted = false;
        }
        result
    }

    fn check_line_length(&self, required: usize) -> Result<(), YEncWriteError> {
        if required > self.options.max_line_length {
            Err(YEncWriteError::LineTooLong {
                required,
                limit: self.options.max_line_length,
            })
        } else {
            Ok(())
        }
    }

    fn create_header(
        &self,
        header: &YEncHeader,
        file_checksum: Option<u32>,
    ) -> Result<Vec<u8>, YEncWriteError> {
        // An escape pair may extend a payload line by one byte.
        self.check_line_length(header.line_length() + 1)?;
        if header.decoded_size() > self.options.max_decoded_size {
            return Err(YEncWriteError::BlockTooLarge {
                size: header.decoded_size(),
                limit: self.options.max_decoded_size,
            });
        }
        self.check_line_length(trailer_text(header, 0, file_checksum).len())?;

        let mut prefix = String::from("=ybegin ");
        if let Some(part) = header.part_number() {
            prefix.push_str(&format!("part={part} "));
            if let Some(total) = header.total_parts() {
                prefix.push_str(&format!("total={total} "));
            }
        }
        prefix.push_str(&format!(
            "line={} size={} name=",
            header.line_length(),
            header.size()
        ));
        let name = self
            .options
            .file_name_charset
            .encode(header.file_name())
            .map_err(YEncWriteError::FileNameEncoding)?;
        self.check_line_length(prefix.len() + name.len())?;
        if let Some(offset) = name
            .iter()
            .position(|byte| matches!(byte, 0 | b'\r' | b'\n'))
        {
            return Err(YEncWriteError::InvalidEncodedFileName { offset });
        }
        let newline = self.options.new_line_format.as_bytes();
        let mut begin = prefix.into_bytes();
        begin.extend_from_slice(&name);
        begin.extend_from_slice(newline);
        if let Some(range) = header.range() {
            let part = format!("=ypart begin={} end={}", range.start(), range.end());
            self.check_line_length(part.len())?;
            begin.extend_from_slice(part.as_bytes());
            begin.extend_from_slice(newline);
        }
        Ok(begin)
    }

    fn write_inner<R: Read + ?Sized>(
        &mut self,
        header: &YEncHeader,
        source: &mut R,
        file_checksum: Option<u32>,
        begin: &[u8],
        mut encoder: YEncoder,
    ) -> Result<u32, YEncWriteError> {
        let mut input = [0; 4096];
        let mut output = Vec::new();
        let mut remaining = header.decoded_size();
        self.inner.write_all(begin).map_err(YEncWriteError::Io)?;
        loop {
            let count = if remaining >= input.len() as u64 {
                input.len()
            } else {
                remaining as usize + 1
            };
            let n = match source.read(&mut input[..count]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result.map_err(YEncWriteError::Io)?,
            };
            if n == 0 {
                if remaining != 0 {
                    return Err(YEncWriteError::SourceTooShort {
                        expected: header.decoded_size(),
                        actual: header.decoded_size() - remaining,
                    });
                }
                break;
            }
            if n as u64 > remaining {
                return Err(YEncWriteError::SourceTooLong {
                    expected: header.decoded_size(),
                });
            }
            remaining -= n as u64;
            encoder.encode(&input[..n], &mut output);
            self.inner.write_all(&output).map_err(YEncWriteError::Io)?;
            output.clear();
        }
        encoder.finish(&mut output);
        self.inner.write_all(&output).map_err(YEncWriteError::Io)?;

        let checksum = !encoder.checksum();
        let whole_file = header
            .range()
            .is_none_or(|range| *range.start() == 1 && *range.end() == header.size());
        if let Some(expected) = file_checksum
            && whole_file
            && expected != checksum
        {
            return Err(YEncWriteError::ChecksumMismatch {
                expected,
                actual: checksum,
            });
        }
        let mut trailer = trailer_text(header, checksum, file_checksum).into_bytes();
        trailer.extend_from_slice(self.options.new_line_format.as_bytes());
        self.inner.write_all(&trailer).map_err(YEncWriteError::Io)?;
        Ok(checksum)
    }
}

fn trailer_text(header: &YEncHeader, checksum: u32, file_checksum: Option<u32>) -> String {
    let mut text = format!("=yend size={}", header.decoded_size());
    if let Some(part) = header.part_number() {
        text.push_str(&format!(" part={part}"));
        if let Some(total) = header.total_parts() {
            text.push_str(&format!(" total={total}"));
        }
        text.push_str(&format!(" pcrc32={checksum:08x}"));
        if let Some(checksum) = file_checksum {
            text.push_str(&format!(" crc32={checksum:08x}"));
        }
    } else {
        text.push_str(&format!(" crc32={checksum:08x}"));
    }
    text
}

/// A preflight or streaming failure while writing a yEnc block.
#[derive(Debug)]
pub enum YEncWriteError {
    /// The maximum physical line length is outside 1 through `i32::MAX - 2`.
    InvalidMaxLineLength(usize),
    /// Invalid payload line length or output newline format.
    Encoder(EncoderOptionsError),
    /// A control line or the payload's worst-case line exceeds the limit.
    LineTooLong {
        /// The required length, excluding newline bytes.
        required: usize,
        /// The configured maximum line length.
        limit: usize,
    },
    /// The decoded block size exceeds its configured limit.
    BlockTooLarge {
        /// The declared decoded block size.
        size: u64,
        /// The configured maximum decoded block size.
        limit: u64,
    },
    /// The filename cannot be encoded without replacement.
    FileNameEncoding(CharsetError),
    /// The encoded filename contains NUL, CR, or LF at this byte offset.
    InvalidEncodedFileName {
        /// The byte offset within the encoded filename.
        offset: usize,
    },
    /// EOF was reached before the declared block size.
    SourceTooShort {
        /// The declared decoded block size.
        expected: u64,
        /// The number of decoded bytes read before EOF.
        actual: u64,
    },
    /// At least one byte follows the declared block size.
    SourceTooLong {
        /// The declared decoded block size.
        expected: u64,
    },
    /// A supplied whole-file CRC differs from the complete source's CRC.
    ChecksumMismatch {
        /// The supplied finalized whole-file CRC-32.
        expected: u32,
        /// The computed finalized CRC-32.
        actual: u32,
    },
    /// A source or destination I/O error.
    Io(io::Error),
    /// A prior operation failed after I/O began.
    Faulted,
}

impl fmt::Display for YEncWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMaxLineLength(length) => write!(
                f,
                "yEnc maximum line length must be 1 through {MAX_YENC_LINE_LENGTH}, got {length}"
            ),
            Self::Encoder(error) => fmt::Display::fmt(error, f),
            Self::LineTooLong { required, limit } => {
                write!(
                    f,
                    "yEnc line requires {required} bytes, exceeding the limit of {limit}"
                )
            }
            Self::BlockTooLarge { size, limit } => {
                write!(f, "yEnc block size {size} exceeds the limit of {limit}")
            }
            Self::FileNameEncoding(error) => fmt::Display::fmt(error, f),
            Self::InvalidEncodedFileName { offset } => {
                write!(
                    f,
                    "encoded yEnc filename contains NUL, CR or LF at byte {offset}"
                )
            }
            Self::SourceTooShort { expected, actual } => {
                write!(
                    f,
                    "yEnc source contains {actual} bytes, expected {expected}"
                )
            }
            Self::SourceTooLong { expected } => {
                write!(f, "yEnc source exceeds its declared size of {expected}")
            }
            Self::ChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "yEnc whole-file checksum is {actual:08x}, expected {expected:08x}"
                )
            }
            Self::Io(error) => fmt::Display::fmt(error, f),
            Self::Faulted => f.write_str("yEnc writer cannot be reused after a failed write"),
        }
    }
}

impl Error for YEncWriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Encoder(error) => Some(error),
            Self::FileNameEncoding(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

// Adapted from MimeKit's Encodings/YEncReader.cs, YEncFormat.cs, and YEncOptions.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
};

use crate::charsets::{Charset, CharsetError};

use super::{YDecoder, YEncHeader, YEncHeaderError, yenc_header::MAX_YENC_LINE_LENGTH};

/// Options copied by [`YEncReader::with_options`].
///
/// Readers accept CRLF and LF regardless of the writer's newline setting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct YEncReaderOptions {
    /// Maximum scanned/control/payload line length, excluding the newline.
    /// Defaults to 16384; valid values are 1 through `i32::MAX - 2`.
    pub max_line_length: usize,
    /// Maximum decoded size of each block, not the entire multipart file.
    /// Defaults to 1 GiB. Zero permits only empty single-part blocks.
    pub max_decoded_size: u64,
    /// Filename charset; defaults to exact Latin-1. Invalid bytes are errors.
    /// The format does not declare a charset; select the writer's encoding.
    pub file_name_charset: Charset,
}

impl Default for YEncReaderOptions {
    fn default() -> Self {
        Self {
            max_line_length: 16 * 1024,
            max_decoded_size: 1024 * 1024 * 1024,
            file_name_charset: Charset::LATIN1,
        }
    }
}

/// A streaming reader for validated single-part and multipart yEnc blocks.
///
/// Call [`read_next`](Self::read_next), then [`decode_to`](Self::decode_to) for
/// each block. To discard a block while still validating it, decode to
/// [`io::sink`]. Surrounding text is skipped, subject to the physical-line limit.
///
/// Output is provisional until decoding succeeds; discard it on failure.
/// Missing checksums are accepted, while present checksums are verified when
/// they cover this block. A whole-file checksum of a partial range is unverified
/// metadata. This reader does not assemble parts, interpret MIME, or undo NNTP
/// dot-stuffing; supply the article body after transport decoding.
///
/// Sources need not seek. The reader buffers beyond the current block, so do
/// not access the source directly while reading. Use [`into_parts`](Self::into_parts)
/// to recover unread buffered bytes. Dropping does not decode or validate a
/// pending block; owned and borrowed sources follow normal Rust ownership.
///
/// ```
/// use mimecraft::encodings::YEncReader;
///
/// let input = b"=ybegin line=128 size=9 name=test.bin\n\
///               [\\]^_`abc\n=yend size=9 crc32=cbf43926\n";
/// let mut reader = YEncReader::new(input.as_slice());
/// let mut decoded = Vec::new();
/// assert!(reader.read_next()?);
/// assert_eq!(reader.header().unwrap().file_name(), "test.bin");
/// reader.decode_to(&mut decoded)?;
/// assert_eq!(decoded, b"123456789");
/// assert_eq!(reader.checksum(), Some(0xcbf43926));
/// assert!(!reader.read_next()?);
/// # Ok::<(), mimecraft::encodings::YEncReadError>(())
/// ```
#[derive(Debug)]
pub struct YEncReader<R> {
    inner: BufReader<R>,
    options: YEncReaderOptions,
    line: Vec<u8>,
    header: Option<YEncHeader>,
    decoded_size: u64,
    checksum: Option<u32>,
    expected_checksum: Option<u32>,
    expected_part_checksum: Option<u32>,
    pending: bool,
    faulted: bool,
    eof: bool,
}

impl<R: Read> YEncReader<R> {
    /// Wraps a source with default options without reading it.
    pub fn new(inner: R) -> Self {
        Self::from_options(inner, YEncReaderOptions::default())
    }

    /// Wraps a source and copies options without reading it.
    ///
    /// # Errors
    ///
    /// Rejects a maximum line length outside 1 through `i32::MAX - 2`.
    pub fn with_options(inner: R, options: &YEncReaderOptions) -> Result<Self, YEncReadError> {
        if !(1..=MAX_YENC_LINE_LENGTH).contains(&options.max_line_length) {
            return Err(YEncReadError::InvalidMaxLineLength(options.max_line_length));
        }
        Ok(Self::from_options(inner, *options))
    }

    fn from_options(inner: R, options: YEncReaderOptions) -> Self {
        Self {
            inner: BufReader::with_capacity(4096, inner),
            options,
            line: Vec::new(),
            header: None,
            decoded_size: 0,
            checksum: None,
            expected_checksum: None,
            expected_part_checksum: None,
            pending: false,
            faulted: false,
            eof: false,
        }
    }

    /// Returns the current validated header, retained after decoding.
    ///
    /// Returns `None` before the first successful header read or after EOF.
    /// A failed header read never publishes partially validated metadata.
    pub fn header(&self) -> Option<&YEncHeader> {
        self.header.as_ref()
    }

    /// Returns the current block's decoded byte count, not necessarily bytes
    /// delivered to the destination. This does not imply successful validation.
    pub fn decoded_size(&self) -> u64 {
        self.decoded_size
    }

    /// Returns the finalized block CRC-32 only after successful decoding.
    pub fn checksum(&self) -> Option<u32> {
        self.checksum
    }

    /// Returns the trailer's whole-file CRC-32 after successful decoding.
    ///
    /// For partial ranges this remains unverified metadata.
    pub fn expected_checksum(&self) -> Option<u32> {
        self.expected_checksum
    }

    /// Returns the trailer's verified part CRC-32 after successful decoding.
    pub fn expected_part_checksum(&self) -> Option<u32> {
        self.expected_part_checksum
    }

    /// Returns whether a previous operation failed after reading began.
    pub fn is_faulted(&self) -> bool {
        self.faulted
    }

    /// Returns the source. Reading it directly would bypass buffered input.
    pub fn get_ref(&self) -> &R {
        self.inner.get_ref()
    }

    /// Returns the source and unread read-ahead bytes, in that order.
    ///
    /// Consume the returned bytes before reading the source again. Does not
    /// validate a pending block or recover already consumed malformed input.
    pub fn into_parts(self) -> (R, Vec<u8>) {
        let buffered = self.inner.buffer().to_vec();
        (self.inner.into_inner(), buffered)
    }

    /// Finds the next block, skipping surrounding text; returns `false` at EOF.
    ///
    /// Clears metadata/checksums from the preceding completed block. Decode the
    /// current block before advancing. Repeated EOF calls do not read again.
    ///
    /// # Errors
    ///
    /// Malformed/incomplete headers, limit violations, or source errors
    /// permanently fault the reader. Calling with a pending block is a state
    /// error that leaves the reader usable. Interrupted reads are retried.
    pub fn read_next(&mut self) -> Result<bool, YEncReadError> {
        if self.faulted {
            return Err(YEncReadError::Faulted);
        }
        if self.pending {
            return Err(YEncReadError::PendingBlock);
        }
        self.header = None;
        self.decoded_size = 0;
        self.checksum = None;
        self.expected_checksum = None;
        self.expected_part_checksum = None;
        self.faulted = true;
        let result = self.read_next_inner();
        if result.is_ok() {
            self.faulted = false;
        }
        result
    }

    fn read_next_inner(&mut self) -> Result<bool, YEncReadError> {
        while self.read_line()? {
            if !self.line.starts_with(b"=ybegin ") {
                continue;
            }
            let fields = Fields::parse(&self.line, 8, true)?;
            let name = self
                .options
                .file_name_charset
                .decode(fields.required("name")?)
                .map_err(YEncReadError::FileNameDecoding)?;
            let size = fields.number("size", true)?;
            let length = usize::try_from(fields.number("line", true)?)
                .map_err(|_| YEncReadError::InvalidField("line"))?;
            let part_number = fields.integer("part", false)?;
            let total_parts = fields.integer("total", false)?;
            let has_part = fields.contains("part");
            let mut header = YEncHeader::new(&name, size)
                .and_then(|header| header.with_line_length(length))
                .map_err(YEncReadError::Header)?;

            if has_part {
                if !self.read_line()? || !self.line.starts_with(b"=ypart ") {
                    return Err(YEncReadError::MissingPartHeader);
                }
                let fields = Fields::parse(&self.line, 7, false)?;
                let begin = fields.number("begin", true)?;
                let end = fields.number("end", true)?;
                header = header
                    .with_part(
                        part_number,
                        (total_parts != 0).then_some(total_parts),
                        begin..=end,
                    )
                    .map_err(YEncReadError::Header)?;
            } else if total_parts != 0 {
                return Err(YEncReadError::InvalidField("total"));
            }
            if header.decoded_size() > self.options.max_decoded_size {
                return Err(YEncReadError::BlockTooLarge {
                    size: header.decoded_size(),
                    limit: self.options.max_decoded_size,
                });
            }
            if header.line_length() > self.options.max_line_length {
                return Err(YEncReadError::LineTooLong {
                    limit: self.options.max_line_length,
                });
            }
            self.header = Some(header);
            self.pending = true;
            return Ok(true);
        }
        Ok(false)
    }

    fn read_line(&mut self) -> Result<bool, YEncReadError> {
        self.line.clear();
        loop {
            if self.eof {
                self.finish_line();
                return Ok(!self.line.is_empty());
            }
            let input = match self.inner.fill_buf() {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result.map_err(YEncReadError::Io)?,
            };
            if input.is_empty() {
                self.eof = true;
                continue;
            }
            let end = memchr::memchr(b'\n', input);
            let count = end.unwrap_or(input.len());
            let content = &input[..count];
            let remaining = self.options.max_line_length.saturating_sub(self.line.len());
            // Reserve one extra byte only for a CR that may be followed by LF,
            // including when the two newline bytes arrive in different reads.
            if content.len() > remaining
                && !(self.line.len() <= self.options.max_line_length
                    && content.len() == remaining + 1
                    && content.last() == Some(&b'\r'))
            {
                return Err(YEncReadError::LineTooLong {
                    limit: self.options.max_line_length,
                });
            }
            self.line.extend_from_slice(content);
            self.inner.consume(count + usize::from(end.is_some()));
            if end.is_some() {
                self.finish_line();
                return Ok(true);
            }
        }
    }

    fn finish_line(&mut self) {
        if self.line.last() == Some(&b'\r') {
            self.line.pop();
        }
    }

    /// Decodes and validates the pending block, without flushing the destination.
    ///
    /// Discard output if this fails: it may be incomplete or corrupt. Missing
    /// CRC fields are accepted; advertised part CRCs and whole-file CRCs covering
    /// this block are checked. The destination remains borrowed throughout.
    ///
    /// # Errors
    ///
    /// Syntax, size, checksum, limit, source, and destination failures permanently
    /// fault the reader. Calling without a pending header is a state error that
    /// leaves the reader usable. Interrupted I/O and short writes are handled.
    pub fn decode_to<W: Write + ?Sized>(
        &mut self,
        destination: &mut W,
    ) -> Result<(), YEncReadError> {
        if self.faulted {
            return Err(YEncReadError::Faulted);
        }
        if !self.pending {
            return Err(YEncReadError::NoPendingBlock);
        }
        self.faulted = true;
        let result = self.decode_inner(destination);
        if result.is_ok() {
            self.faulted = false;
        }
        result
    }

    fn decode_inner<W: Write + ?Sized>(
        &mut self,
        destination: &mut W,
    ) -> Result<(), YEncReadError> {
        let mut decoder = YDecoder::for_payload();
        let mut output = Vec::new();
        while self.read_line()? {
            if self.line.starts_with(b"=yend ") {
                return self.validate_trailer(&decoder);
            }
            self.decode_line(&mut decoder, &mut output)?;
            destination.write_all(&output).map_err(YEncReadError::Io)?;
            output.clear();
        }
        Err(YEncReadError::MissingTrailer)
    }

    fn decode_line(
        &mut self,
        decoder: &mut YDecoder,
        output: &mut Vec<u8>,
    ) -> Result<(), YEncReadError> {
        let header = self.header.as_ref().expect("decoding requires a header");
        if self.line.starts_with(b"=y") {
            return Err(YEncReadError::UnexpectedControlLine);
        }
        let too_long = || YEncReadError::PayloadLineTooLong {
            length: self.line.len(),
            limit: header.line_length(),
        };
        if self.line.len() > header.line_length() + 1 {
            return Err(too_long());
        }
        let mut final_escape = false;
        let mut index = 0;
        while index < self.line.len() {
            if matches!(self.line[index], 0 | b'\r') {
                return Err(YEncReadError::InvalidPayloadByte { offset: index });
            }
            if self.line[index] == b'=' {
                final_escape = index + 2 == self.line.len();
                let offset = index;
                index += 1;
                if index == self.line.len() || matches!(self.line[index], 0 | b'\r') {
                    return Err(YEncReadError::InvalidEscape { offset });
                }
            }
            index += 1;
        }
        if self.line.len() > header.line_length() && !final_escape {
            return Err(too_long());
        }
        decoder.decode(&self.line, output);
        decoder.decode(b"\n", output);
        if output.len() as u64 > header.decoded_size() - self.decoded_size {
            return Err(YEncReadError::PayloadTooLarge {
                expected: header.decoded_size(),
            });
        }
        self.decoded_size += output.len() as u64;
        Ok(())
    }

    fn validate_trailer(&mut self, decoder: &YDecoder) -> Result<(), YEncReadError> {
        let fields = Fields::parse(&self.line, 6, false)?;
        let header = self.header.as_ref().expect("decoding requires a header");
        let size = fields.number("size", true)?;
        if size != self.decoded_size || self.decoded_size != header.decoded_size() {
            return Err(YEncReadError::SizeMismatch {
                header: header.decoded_size(),
                decoded: self.decoded_size,
                trailer: size,
            });
        }
        if (header.part_number().is_some() || fields.contains("part"))
            && fields.integer("part", true)? != header.part_number().unwrap_or(0)
        {
            return Err(YEncReadError::InvalidField("part"));
        }
        if fields.contains("total")
            && fields.integer("total", true)? != header.total_parts().unwrap_or(0)
        {
            return Err(YEncReadError::InvalidField("total"));
        }
        let actual = !decoder.checksum();
        let expected = fields.checksum("crc32")?;
        let part = fields.checksum("pcrc32")?;
        let whole_file = header
            .range()
            .is_none_or(|range| *range.start() == 1 && *range.end() == header.size());
        for (field, expected) in [("pcrc32", part), ("crc32", expected.filter(|_| whole_file))] {
            if let Some(expected) = expected
                && expected != actual
            {
                return Err(YEncReadError::ChecksumMismatch {
                    field,
                    expected,
                    actual,
                });
            }
        }
        self.checksum = Some(actual);
        self.expected_checksum = expected;
        self.expected_part_checksum = part;
        self.pending = false;
        Ok(())
    }
}

struct Fields<'a>(HashMap<&'a [u8], &'a [u8]>);

impl<'a> Fields<'a> {
    fn parse(line: &'a [u8], mut index: usize, header: bool) -> Result<Self, YEncReadError> {
        let mut fields = HashMap::new();
        while index < line.len() {
            if line[index] == b' ' {
                index += 1;
                continue;
            }
            let end = memchr::memchr(b' ', &line[index..]).map_or(line.len(), |end| index + end);
            let equals = memchr::memchr(b'=', &line[index..end])
                .filter(|&equals| equals != 0)
                .ok_or(YEncReadError::MalformedField { offset: index })?
                + index;
            let key = &line[index..equals];
            let filename = header && key == b"name";
            let value = if filename {
                let mut value = &line[equals + 1..];
                while value.first() == Some(&b' ') {
                    value = &value[1..];
                }
                while value.last() == Some(&b' ') {
                    value = &value[..value.len() - 1];
                }
                value
            } else {
                &line[equals + 1..end]
            };
            if value.is_empty() {
                return Err(YEncReadError::MalformedField { offset: index });
            }
            if fields.insert(key, value).is_some() {
                return Err(YEncReadError::DuplicateField { offset: index });
            }
            if filename {
                break;
            }
            index = end;
        }
        Ok(Self(fields))
    }

    fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name.as_bytes())
    }

    fn required(&self, name: &'static str) -> Result<&'a [u8], YEncReadError> {
        self.0
            .get(name.as_bytes())
            .copied()
            .ok_or(YEncReadError::MissingField(name))
    }

    fn number(&self, name: &'static str, required: bool) -> Result<u64, YEncReadError> {
        let Some(value) = self.0.get(name.as_bytes()) else {
            return if required {
                Err(YEncReadError::MissingField(name))
            } else {
                Ok(0)
            };
        };
        value.iter().try_fold(0u64, |number, &byte| {
            if !byte.is_ascii_digit() {
                return Err(YEncReadError::InvalidField(name));
            }
            number
                .checked_mul(10)
                .and_then(|number| number.checked_add(u64::from(byte - b'0')))
                .ok_or(YEncReadError::InvalidField(name))
        })
    }

    fn integer(&self, name: &'static str, required: bool) -> Result<u32, YEncReadError> {
        u32::try_from(self.number(name, required)?).map_err(|_| YEncReadError::InvalidField(name))
    }

    fn checksum(&self, name: &'static str) -> Result<Option<u32>, YEncReadError> {
        let Some(value) = self.0.get(name.as_bytes()) else {
            return Ok(None);
        };
        if value.len() != 8 {
            return Err(YEncReadError::InvalidField(name));
        }
        let mut checksum = 0;
        for &byte in *value {
            let digit = super::hex::hex_rank(byte).ok_or(YEncReadError::InvalidField(name))?;
            checksum = (checksum << 4) | u32::from(digit);
        }
        Ok(Some(checksum))
    }
}

/// A state, format, limit, or I/O error while reading a yEnc block.
#[derive(Debug)]
pub enum YEncReadError {
    /// The maximum physical line length is outside 1 through `i32::MAX - 2`.
    InvalidMaxLineLength(usize),
    /// A scanned line or declared nominal width exceeds the physical limit.
    LineTooLong {
        /// The configured physical-line limit, excluding the newline.
        limit: usize,
    },
    /// The declared decoded block size exceeds the configured limit.
    BlockTooLarge {
        /// The decoded size of this block.
        size: u64,
        /// The configured per-block maximum.
        limit: u64,
    },
    /// A required control field is missing.
    MissingField(&'static str),
    /// A numeric/checksum field is invalid, overflowing, or inconsistent.
    InvalidField(&'static str),
    /// A control token lacks a name, equals sign, or nonempty value.
    MalformedField {
        /// Byte offset within the control line.
        offset: usize,
    },
    /// A control line repeats a field, including an unknown field.
    DuplicateField {
        /// Byte offset of the repeated field within the control line.
        offset: usize,
    },
    /// The parsed metadata is invalid.
    Header(YEncHeaderError),
    /// The filename is not valid in the selected charset.
    FileNameDecoding(CharsetError),
    /// A multipart begin line is not immediately followed by a part header.
    MissingPartHeader,
    /// The pending block reaches EOF without a trailer.
    MissingTrailer,
    /// An unexpected `=y` control line occurs in the payload.
    UnexpectedControlLine,
    /// A payload line exceeds its nominal width without the final-escape exception.
    PayloadLineTooLong {
        /// The encoded line length, excluding the newline.
        length: usize,
        /// The nominal payload width.
        limit: usize,
    },
    /// An unescaped NUL or CR occurs in the payload.
    InvalidPayloadByte {
        /// Byte offset within the payload line.
        offset: usize,
    },
    /// An escape is incomplete or followed by NUL or CR.
    InvalidEscape {
        /// Byte offset of the equals sign within the payload line.
        offset: usize,
    },
    /// Decoding this line would exceed the declared block size.
    PayloadTooLarge {
        /// The declared decoded block size.
        expected: u64,
    },
    /// The header, decoded count, and trailer sizes disagree.
    SizeMismatch {
        /// The header's decoded block size.
        header: u64,
        /// The number of decoded bytes.
        decoded: u64,
        /// The trailer's decoded size.
        trailer: u64,
    },
    /// An advertised checksum covering this block does not match.
    ChecksumMismatch {
        /// The field name: `crc32` or `pcrc32`.
        field: &'static str,
        /// The advertised finalized CRC-32.
        expected: u32,
        /// The computed finalized CRC-32.
        actual: u32,
    },
    /// A source or destination I/O error.
    Io(io::Error),
    /// Decode the current block before advancing to another header.
    PendingBlock,
    /// Read a header before decoding a block.
    NoPendingBlock,
    /// A prior operation failed after reading began.
    Faulted,
}

impl fmt::Display for YEncReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMaxLineLength(length) => write!(
                f,
                "yEnc maximum line length must be 1 through {MAX_YENC_LINE_LENGTH}, got {length}"
            ),
            Self::LineTooLong { limit } => {
                write!(f, "yEnc input exceeds the line limit of {limit}")
            }
            Self::BlockTooLarge { size, limit } => {
                write!(f, "yEnc block size {size} exceeds the limit of {limit}")
            }
            Self::MissingField(name) => write!(f, "missing yEnc {name} field"),
            Self::InvalidField(name) => write!(f, "invalid yEnc {name} field"),
            Self::MalformedField { offset } => {
                write!(f, "malformed yEnc field at control-line byte {offset}")
            }
            Self::DuplicateField { offset } => {
                write!(f, "duplicate yEnc field at control-line byte {offset}")
            }
            Self::Header(error) => fmt::Display::fmt(error, f),
            Self::FileNameDecoding(error) => fmt::Display::fmt(error, f),
            Self::MissingPartHeader => f.write_str("missing yEnc part header"),
            Self::MissingTrailer => f.write_str("missing yEnc trailer"),
            Self::UnexpectedControlLine => f.write_str("unexpected yEnc control line"),
            Self::PayloadLineTooLong { length, limit } => write!(
                f,
                "invalid yEnc payload line length {length}, nominal width {limit}"
            ),
            Self::InvalidPayloadByte { offset } => write!(
                f,
                "unescaped critical byte at yEnc payload-line byte {offset}"
            ),
            Self::InvalidEscape { offset } => write!(
                f,
                "invalid or incomplete yEnc escape at payload-line byte {offset}"
            ),
            Self::PayloadTooLarge { expected } => {
                write!(f, "yEnc payload exceeds its declared size of {expected}")
            }
            Self::SizeMismatch {
                header,
                decoded,
                trailer,
            } => write!(
                f,
                "yEnc block sizes disagree: header {header}, decoded {decoded}, trailer {trailer}"
            ),
            Self::ChecksumMismatch {
                field,
                expected,
                actual,
            } => write!(f, "yEnc {field} is {actual:08x}, expected {expected:08x}"),
            Self::Io(error) => fmt::Display::fmt(error, f),
            Self::PendingBlock => {
                f.write_str("decode the current yEnc block before reading another")
            }
            Self::NoPendingBlock => f.write_str("read a yEnc header before decoding a block"),
            Self::Faulted => f.write_str("yEnc reader cannot be reused after a failed operation"),
        }
    }
}

impl Error for YEncReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Header(error) => Some(error),
            Self::FileNameDecoding(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

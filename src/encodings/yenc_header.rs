// Adapted from MimeKit's Encodings/YEncHeader.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, ops::RangeInclusive};

pub(super) const MAX_YENC_LINE_LENGTH: usize = i32::MAX as usize - 2;

/// Immutable metadata for a single-part or multipart yEnc block.
///
/// Sizes and ranges describe decoded bytes. Multipart ranges are one-based and
/// inclusive; the source for a writer must contain only that range. Metadata
/// does not imply that content has been validated. A filename is untrusted
/// metadata, never a filesystem path.
///
/// ```
/// use mimecraft::encodings::YEncHeader;
///
/// let header = YEncHeader::new("file.bin", 100)?
///     .with_line_length(60)?
///     .with_part(2, Some(2), 51..=100)?;
/// assert_eq!(header.size(), 100);
/// assert_eq!(header.decoded_size(), 50);
/// # Ok::<(), mimecraft::encodings::YEncHeaderError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YEncHeader {
    file_name: String,
    size: u64,
    line_length: usize,
    part_number: Option<u32>,
    total_parts: Option<u32>,
    range: Option<RangeInclusive<u64>>,
}

impl YEncHeader {
    /// Creates single-part metadata with a nominal payload line length of 128.
    ///
    /// # Errors
    ///
    /// Rejects an empty filename, surrounding ASCII spaces, or NUL, CR, or LF.
    pub fn new(file_name: &str, size: u64) -> Result<Self, YEncHeaderError> {
        if file_name.is_empty()
            || file_name.starts_with(' ')
            || file_name.ends_with(' ')
            || file_name.contains(['\0', '\r', '\n'])
        {
            return Err(YEncHeaderError::InvalidFileName);
        }
        Ok(Self {
            file_name: file_name.to_owned(),
            size,
            line_length: 128,
            part_number: None,
            total_parts: None,
            range: None,
        })
    }

    /// Sets the nominal payload line length, excluding the newline.
    ///
    /// An escape pair may extend a payload line by one byte. Metadata accepts
    /// lengths from 1 through `i32::MAX - 2`; writing requires 60 through 998.
    ///
    /// # Errors
    ///
    /// Returns an error for lengths outside the metadata range.
    pub fn with_line_length(mut self, line_length: usize) -> Result<Self, YEncHeaderError> {
        if !(1..=MAX_YENC_LINE_LENGTH).contains(&line_length) {
            return Err(YEncHeaderError::InvalidLineLength(line_length));
        }
        self.line_length = line_length;
        Ok(self)
    }

    /// Sets multipart metadata; `None` leaves the total part count unspecified.
    ///
    /// # Errors
    ///
    /// Rejects zero part numbers/counts, a part number above a specified total,
    /// and empty or out-of-file ranges. Range positions are one-based.
    pub fn with_part(
        mut self,
        part_number: u32,
        total_parts: Option<u32>,
        range: RangeInclusive<u64>,
    ) -> Result<Self, YEncHeaderError> {
        if part_number == 0 {
            return Err(YEncHeaderError::InvalidPartNumber(part_number));
        }
        if let Some(total_parts) = total_parts
            && (total_parts == 0 || part_number > total_parts)
        {
            return Err(YEncHeaderError::InvalidTotalParts {
                part_number,
                total_parts,
            });
        }
        if range.is_empty() || *range.start() == 0 || *range.end() > self.size {
            return Err(YEncHeaderError::InvalidRange {
                begin: *range.start(),
                end: *range.end(),
                size: self.size,
            });
        }
        self.part_number = Some(part_number);
        self.total_parts = total_parts;
        self.range = Some(range);
        Ok(self)
    }

    /// Returns the untrusted filename, not a filesystem path.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Returns the decoded size of the entire file, not just this part.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Returns the nominal payload line length, excluding the newline.
    pub fn line_length(&self) -> usize {
        self.line_length
    }

    /// Returns the part number, or `None` for a single-part block.
    pub fn part_number(&self) -> Option<u32> {
        self.part_number
    }

    /// Returns the total part count, if specified.
    pub fn total_parts(&self) -> Option<u32> {
        self.total_parts
    }

    /// Returns the one-based inclusive multipart range, if present.
    pub fn range(&self) -> Option<&RangeInclusive<u64>> {
        self.range.as_ref()
    }

    /// Returns the decoded size of this block.
    pub fn decoded_size(&self) -> u64 {
        self.range
            .as_ref()
            .map_or(self.size, |range| range.end() - range.start() + 1)
    }
}

/// Invalid yEnc metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum YEncHeaderError {
    /// The filename is empty, has surrounding spaces, or contains NUL, CR, or LF.
    InvalidFileName,
    /// The nominal line length is outside 1 through `i32::MAX - 2`.
    InvalidLineLength(usize),
    /// A multipart part number is zero.
    InvalidPartNumber(u32),
    /// The total is zero or smaller than the part number.
    InvalidTotalParts {
        /// The requested part number.
        part_number: u32,
        /// The rejected total part count.
        total_parts: u32,
    },
    /// The one-based inclusive range is empty or outside the file.
    InvalidRange {
        /// The requested inclusive start.
        begin: u64,
        /// The requested inclusive end.
        end: u64,
        /// The entire file's decoded size.
        size: u64,
    },
}

impl fmt::Display for YEncHeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFileName => f.write_str(
                "yEnc filenames must be nonempty, without surrounding spaces or NUL, CR, LF",
            ),
            Self::InvalidLineLength(length) => write!(
                f,
                "yEnc metadata line length must be 1 through {MAX_YENC_LINE_LENGTH}, got {length}"
            ),
            Self::InvalidPartNumber(number) => {
                write!(f, "yEnc part numbers must be positive, got {number}")
            }
            Self::InvalidTotalParts {
                part_number,
                total_parts,
            } => write!(
                f,
                "yEnc total part count {total_parts} is invalid for part {part_number}"
            ),
            Self::InvalidRange { begin, end, size } => write!(
                f,
                "yEnc range {begin}..={end} is outside the one-based file range 1..={size}"
            ),
        }
    }
}

impl Error for YEncHeaderError {}

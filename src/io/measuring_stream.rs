// Adapted from MimeKit's IO/MeasuringStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Seek, SeekFrom, Write};

/// A seekable sink that measures output without storing or inspecting its bytes.
///
/// Use this in place of a file or memory buffer to measure serialized headers,
/// encoded bodies, or other output written through [`Write`]. A sequential
/// writer's byte count equals [`len`](Self::len). After seeking, overwrites do
/// not increase length unless they extend the previous end: this measures
/// logical stream length, not the cumulative number of bytes submitted.
///
/// Seeking is restricted to `0..=len()`, matching MimeKit. Resizing adjusts the
/// logical length and clamps position without allocating. All offsets use
/// `u64`, rather than MimeKit's signed counters. Overflow is reported, not
/// wrapped. No bytes are retained, and reading is not supported.
///
/// With a [`super::FilteredWriter`], call its `finish()` before inspecting the
/// measured length so pending bytes, padding, and terminators are included:
///
/// ```
/// use std::io::Write;
/// use mimecraft::{
///     encodings::Base64Encoder,
///     filters::EncoderFilter,
///     io::{FilteredWriter, MeasuringStream},
/// };
///
/// let mut writer =
///     FilteredWriter::new(MeasuringStream::new(), EncoderFilter::new(Base64Encoder::new()));
/// writer.write_all(b"Cat")?;
/// writer.finish()?;
/// assert_eq!(writer.get_ref().len(), 6); // "Q2F0\r\n"
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// Unsupported reading is rejected at compile time rather than at runtime:
///
/// ```compile_fail
/// use std::io::Read;
/// use mimecraft::io::MeasuringStream;
///
/// MeasuringStream::new().read(&mut [0; 1]).unwrap();
/// ```
#[derive(Debug, Default)]
pub struct MeasuringStream {
    position: u64,
    length: u64,
}

impl MeasuringStream {
    /// Creates an empty measuring sink at position zero without allocating.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the logical length, not the cumulative number of bytes written.
    pub fn len(&self) -> u64 {
        self.length
    }

    /// Returns whether the logical length is zero.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Returns the current byte offset.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Sets the logical length and clamps position to the new end if necessary.
    ///
    /// This allocates nothing and accepts every `u64` length, including
    /// `u64::MAX`. Use `set_len(0)` to reset both length and position.
    pub fn set_len(&mut self, length: u64) {
        self.position = self.position.min(length);
        self.length = length;
    }
}

impl Write for MeasuringStream {
    /// Counts the entire input without retaining or inspecting its contents.
    ///
    /// Returns `InvalidInput` if the resulting position would overflow `u64`,
    /// leaving length and position unchanged. Empty writes do nothing.
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let position = self
            .position
            .checked_add(input.len() as u64)
            .ok_or_else(|| invalid_position("measured stream position overflows"))?;
        self.position = position;
        self.length = self.length.max(position);
        Ok(input.len())
    }

    /// Does nothing: this sink has no buffered bytes or underlying device.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for MeasuringStream {
    /// Seeks within `0..=len()` without changing the logical length.
    ///
    /// Before-start, past-end, and overflowing offsets return `InvalidInput`
    /// without moving position. Resize first to make a larger range seekable.
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.length.checked_add_signed(offset),
        };
        // Sanity-check the resultant offset before changing the cursor.
        let position = position
            .filter(|&position| position <= self.length)
            .ok_or_else(|| invalid_position("seek is outside the measuring stream"))?;
        self.position = position;
        Ok(position)
    }

    /// Returns the current position without moving it.
    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}

fn invalid_position(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

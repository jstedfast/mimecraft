// Adapted from MimeKit's IO/BoundedStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom, Write};

use super::{seek_absolute, stream_length};

/// A seekable view of a range in an underlying stream, without copying its data.
///
/// Offsets in this view are relative to [`start_boundary`](Self::start_boundary).
/// The optional [`end_boundary`](Self::end_boundary) is an exclusive offset in
/// the underlying stream. `None` leaves the end unbounded. Construction performs
/// no I/O, and does not verify that either boundary exists in the source.
///
/// Implements [`Read`] when the source supports `Read + Seek`, and [`Write`]
/// when it supports `Write + Seek`. Unlike MimeKit's nonseekable fallback, this
/// adapter requires seeking, so a nonzero start always identifies a real offset.
/// It resynchronizes the source before I/O if direct access moved its cursor.
///
/// Seeks cannot precede the start or pass a configured end. `SeekFrom::End`
/// always uses the view's end, including for zero/positive offsets; it does not
/// use MimeKit's special case that delegates nonnegative offsets to the source.
/// With no configured end, seeking past source EOF is allowed if the source
/// supports it. EOF is not cached, so later reads can observe source growth.
///
/// Owns the source, which may itself be a mutable reference. Dropping the view
/// does not flush or restore source position, and does not close a borrowed
/// source. Resizing, nonseekable sources, and async I/O are not implemented.
/// This is a range adapter, not an access-control boundary: direct source access
/// deliberately bypasses its limits.
///
/// ```
/// use std::io::{Cursor, Read, Seek, SeekFrom};
/// use mimecraft::io::BoundedStream;
///
/// let mut source = Cursor::new(b"prefixBODYsuffix");
/// let mut body = BoundedStream::new(&mut source, 6, Some(10))?;
/// let mut text = String::new();
/// body.read_to_string(&mut text)?;
/// assert_eq!(text, "BODY");
/// assert_eq!(body.seek(SeekFrom::End(0))?, 4);
/// body.rewind()?;
/// assert_eq!(body.position(), 0);
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// Seeking is required at compile time, even for a read-only source:
///
/// ```compile_fail
/// use mimecraft::io::BoundedStream;
///
/// // Use a Cursor to add seeking to a byte-slice reader.
/// let stream = BoundedStream::new(&b"body"[..], 0, Some(4));
/// ```
#[derive(Debug)]
pub struct BoundedStream<R> {
    inner: R,
    start: u64,
    end: Option<u64>,
    position: u64,
}

impl<R: Seek> BoundedStream<R> {
    /// Creates a view starting at logical position zero without performing I/O.
    ///
    /// `start` and `end` are absolute offsets in the supplied source, not offsets
    /// from its current cursor. `end` is exclusive; `None` means unbounded.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] if `end` precedes `start`.
    pub fn new(inner: R, start: u64, end: Option<u64>) -> io::Result<Self> {
        if end.is_some_and(|end| end < start) {
            return Err(invalid_position("end boundary precedes start boundary"));
        }
        Ok(Self {
            inner,
            start,
            end,
            position: 0,
        })
    }

    /// Returns the absolute source offset at which this view begins.
    pub fn start_boundary(&self) -> u64 {
        self.start
    }

    /// Returns the exclusive absolute source end, or `None` if unbounded.
    pub fn end_boundary(&self) -> Option<u64> {
        self.end
    }

    /// Returns the logical position relative to the start, without I/O.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Returns the underlying source.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Returns mutable access to the source, bypassing the view's limits.
    ///
    /// Moving its cursor does not change this view's logical position. The next
    /// read/write that reaches the source, or explicit seek, restores its offset.
    /// Changes to source contents or length are visible through the view.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Returns the source without flushing, seeking, or performing other I/O.
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Returns the configured range length or the current source length minus start.
    ///
    /// A fixed end gives `end - start` without I/O, even if the source is shorter.
    /// An unbounded view queries the source end and restores its original cursor
    /// on success. No EOF-derived length is cached. This view's logical position
    /// is not changed.
    ///
    /// # Errors
    ///
    /// Propagates query/restore failures. Such failures may leave the source
    /// cursor moved. A source end before this view's start returns
    /// [`io::ErrorKind::InvalidInput`] rather than a negative length.
    pub fn len(&mut self) -> io::Result<u64> {
        if let Some(end) = self.end {
            return Ok(end - self.start);
        }
        let end = stream_length(&mut self.inner)?;
        end.checked_sub(self.start)
            .ok_or_else(|| invalid_position("source end precedes start boundary"))
    }

    /// Returns whether [`len`](Self::len) is zero, with the same I/O and errors.
    pub fn is_empty(&mut self) -> io::Result<bool> {
        self.len().map(|length| length == 0)
    }

    fn synchronize(&mut self) -> io::Result<()> {
        // Make sure the source stream is in the expected position. Access
        // through get_mut() can have moved it independently of this view.
        let absolute = self.start + self.position;
        if self.inner.stream_position()? != absolute {
            seek_absolute(&mut self.inner, absolute)?;
        }
        Ok(())
    }
}

impl<R: Read + Seek> Read for BoundedStream<R> {
    /// Reads at most the bytes remaining before the exclusive end boundary.
    ///
    /// Empty reads and reads at a fixed end perform no underlying I/O.
    /// A source EOF before the configured end is returned normally, without
    /// shortening the range or preventing subsequent reads.
    ///
    /// Errors (including `Interrupted`) propagate without advancing the logical
    /// position. Successful short reads advance it only by the returned count.
    /// A nonempty unbounded read at absolute `u64::MAX` returns `InvalidInput`,
    /// since advancing the source offset would overflow.
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let remaining = self.end.unwrap_or(u64::MAX) - (self.start + self.position);
        // At the fixed end of the stream, we cannot read any more data.
        if remaining == 0 {
            return if self.end.is_some() {
                Ok(0)
            } else {
                Err(invalid_position("stream position would overflow"))
            };
        }
        let count = output
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        self.synchronize()?;
        let read = self.inner.read(&mut output[..count])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl<R: Write + Seek> Write for BoundedStream<R> {
    /// Writes within the configured range, without flushing.
    ///
    /// Like MimeKit, rejects an entire request that would cross the end, rather
    /// than silently clipping it. Boundary/offset overflow returns `InvalidInput`
    /// before any source I/O. Empty writes are no-ops, including at the end.
    ///
    /// Source short writes are reported and advance position by only that count.
    /// Errors, including `Interrupted`, propagate without advancing position.
    /// A source zero write is returned as zero; `write_all` turns it into
    /// `WriteZero`. Failed I/O can leave the underlying source cursor moved;
    /// the next operation resynchronizes it to this view's position.
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        let end = (self.start + self.position)
            .checked_add(input.len() as u64)
            .ok_or_else(|| invalid_position("stream position would overflow"))?;
        // If the whole request does not fit, do not write even a prefix.
        if self.end.is_some_and(|boundary| end > boundary) {
            return Err(invalid_position("write would pass the end boundary"));
        }
        self.synchronize()?;
        let written = self.inner.write(input)?;
        self.position += written as u64;
        Ok(written)
    }

    /// Flushes the underlying writer without changing logical position.
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<R: Seek> Seek for BoundedStream<R> {
    /// Seeks relative to this view's start, position, or end.
    ///
    /// End-relative seeks use the configured end or query the source's length
    /// for an unbounded view. Invalid offsets return `InvalidInput`. Successful
    /// seeks, even to the current logical position, synchronize the source.
    ///
    /// Logical position changes only on success. Underlying failures (or failed
    /// unbounded end queries) may move the source cursor; later I/O resynchronizes
    /// it. A source that reports a different offset than requested returns
    /// `InvalidData`, rather than allowing an out-of-range read/write.
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.len()?.checked_add_signed(offset),
        }
        .ok_or_else(|| invalid_position("seek is before the start or overflows"))?;
        let absolute = self
            .start
            .checked_add(position)
            .ok_or_else(|| invalid_position("stream position would overflow"))?;
        // Sanity-check the resulting offset before moving the source.
        if self.end.is_some_and(|end| absolute > end) {
            return Err(invalid_position("seek would pass the end boundary"));
        }
        seek_absolute(&mut self.inner, absolute)?;
        self.position = position;
        Ok(position)
    }

    /// Returns the logical position without querying or moving the source.
    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}

fn invalid_position(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

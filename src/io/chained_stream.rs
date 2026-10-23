// Adapted from MimeKit's IO/ChainedStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom, Write};

use super::{seek_absolute, stream_length};

/// Seekable streams presented as one continuous stream, without copying data.
///
/// Reads progress through children in insertion order. Writes preserve each
/// non-final child's length, spilling into the next; the final child may grow
/// if its own writer permits it. Seeks use the concatenation of full child
/// lengths and cannot pass either end of that concatenation.
///
/// Each child is addressed from its own offset zero, independent of its cursor
/// when added. This deliberately differs from MimeKit's initial reads from
/// existing cursors, keeping logical offsets consistent with seeking. Use
/// [`super::BoundedStream`] for a child that represents only a source range.
/// Child lengths must stay stable except for writes through this chain. After
/// external length changes, explicitly seek to recompute the logical location.
///
/// Requires seekable children; [`Read`] and [`Write`] additionally require those
/// traits on the child type. A single read/write may be short at a child boundary.
/// Unlike MimeKit's fill-buffer loop, it returns immediately after transferring
/// bytes, so a later child's error cannot hide already-consumed progress.
/// `read_exact`, `read_to_end`, and `write_all` traverse boundaries normally.
///
/// Owns the children, which may be mutable references. Neither dropping nor
/// dismantling the chain flushes or rewinds them; borrowed sources stay open.
/// Nonseekable chaining, resizing, and async I/O are not implemented.
///
/// ```
/// use std::io::{Cursor, Read, Seek, SeekFrom};
/// use mimecraft::io::ChainedStream;
///
/// let mut chain = ChainedStream::new();
/// chain.add(Cursor::new(&b"header\r\n\r\n"[..]));
/// chain.add(Cursor::new(&b"body"[..]));
/// let mut bytes = Vec::new();
/// chain.read_to_end(&mut bytes)?;
/// assert_eq!(bytes, b"header\r\n\r\nbody");
/// chain.seek(SeekFrom::End(-4))?;
/// let mut body = [0; 4];
/// chain.read_exact(&mut body)?;
/// assert_eq!(&body, b"body");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct ChainedStream<R> {
    streams: Vec<R>,
    current: usize,
    offset: u64,
    position: u64,
}

impl<R: Seek> ChainedStream<R> {
    /// Creates an empty chain at position zero, without I/O.
    pub fn new() -> Self {
        Self::from(Vec::new())
    }

    /// Appends a child without seeking it or changing logical position.
    ///
    /// Appending after EOF makes the new child readable. Its initial cursor is
    /// ignored: it will be read from offset zero when reached.
    pub fn add(&mut self, stream: R) {
        self.streams.push(stream);
    }

    /// Returns the current logical position without I/O.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Returns the child streams in insertion order.
    ///
    /// Their physical cursors need not match their positions in a fully read
    /// concatenation; the chain positions each child when needed.
    pub fn get_ref(&self) -> &[R] {
        &self.streams
    }

    /// Returns all children without seeking or flushing them.
    pub fn into_inner(self) -> Vec<R> {
        self.streams
    }

    /// Returns the sum of the current child lengths.
    ///
    /// Queries each child's end and restores its previous cursor on success.
    /// An empty chain has length zero. Logical chain position is unchanged.
    ///
    /// # Errors
    ///
    /// Propagates seek/query/restore errors; they may leave child cursors moved.
    /// A total exceeding `u64::MAX` returns [`io::ErrorKind::InvalidInput`].
    /// Subsequent I/O repositions the active child before accessing its data.
    pub fn len(&mut self) -> io::Result<u64> {
        let mut total = 0u64;
        for stream in &mut self.streams {
            total = total
                .checked_add(stream_length(stream)?)
                .ok_or_else(|| invalid_position("combined stream length overflows"))?;
        }
        Ok(total)
    }

    /// Returns whether the combined byte length is zero, with [`len`](Self::len)'s errors.
    ///
    /// This can be true for a nonempty collection of empty children.
    pub fn is_empty(&mut self) -> io::Result<bool> {
        self.len().map(|length| length == 0)
    }

    fn synchronize(&mut self) -> io::Result<()> {
        // MimeKit rewinds intervening children when seeking backward. Here,
        // reposition each child lazily before I/O, including after revisiting it.
        let stream = &mut self.streams[self.current];
        if stream.stream_position()? != self.offset {
            seek_absolute(stream, self.offset)?;
        }
        Ok(())
    }

    fn transfer_limit(&self, requested: usize) -> io::Result<usize> {
        let remaining = (u64::MAX - self.position).min(u64::MAX - self.offset);
        if remaining == 0 {
            return Err(invalid_position("stream position would overflow"));
        }
        Ok(requested.min(usize::try_from(remaining).unwrap_or(usize::MAX)))
    }
}

impl<R: Seek> Default for ChainedStream<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Seek> From<Vec<R>> for ChainedStream<R> {
    fn from(streams: Vec<R>) -> Self {
        Self {
            streams,
            current: 0,
            offset: 0,
            position: 0,
        }
    }
}

impl<R: Seek> FromIterator<R> for ChainedStream<R> {
    fn from_iter<T: IntoIterator<Item = R>>(iter: T) -> Self {
        Self::from(iter.into_iter().collect::<Vec<_>>())
    }
}

impl<R: Read + Seek> Read for ChainedStream<R> {
    /// Reads from one child, skipping exhausted or empty children as needed.
    ///
    /// Empty reads perform no I/O. Empty chains return EOF; adding a child
    /// after EOF allows reading to continue. EOF is not latched.
    ///
    /// Errors, including `Interrupted`, propagate without consuming data in
    /// that call. Completed prior calls remain reflected in logical position.
    /// A child returning EOF before its reported length returns `UnexpectedEof`
    /// rather than silently shifting subsequent logical offsets.
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.streams.is_empty() {
            return Ok(0);
        }
        loop {
            // At the largest representable position, check EOF without trying
            // to read a byte whose successful transfer would overflow it.
            let at_limit = self.position == u64::MAX || self.offset == u64::MAX;
            if !at_limit {
                let count = self.transfer_limit(output.len())?;
                self.synchronize()?;
                let read = self.streams[self.current].read(&mut output[..count])?;
                if read != 0 {
                    self.offset += read as u64;
                    self.position += read as u64;
                    return Ok(read);
                }
            }
            let length = stream_length(&mut self.streams[self.current])?;
            if self.offset < length {
                return Err(if at_limit {
                    invalid_position("stream position would overflow")
                } else {
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "child ended before its reported length",
                    )
                });
            }
            if self.current + 1 == self.streams.len() {
                return Ok(0);
            }
            self.current += 1;
            self.offset = 0;
        }
    }
}

impl<R: Write + Seek> Write for ChainedStream<R> {
    /// Writes to one child, advancing past full non-final children.
    ///
    /// Non-final children are not grown; the last writer decides whether it
    /// can grow. A full non-final writer is flushed before advancing, including
    /// when the previous call filled it exactly. A failed flush is retryable
    /// without duplicating earlier bytes.
    ///
    /// Empty writes perform no I/O. A nonempty write to an empty chain returns
    /// `WriteZero`. Short/zero writes and errors are returned as reported by the
    /// child; no later I/O occurs after bytes have been accepted in this call.
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        if self.streams.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "chain has no writable children",
            ));
        }
        loop {
            let mut count = self.transfer_limit(input.len())?;
            if self.current + 1 < self.streams.len() {
                let length = stream_length(&mut self.streams[self.current])?;
                let left = length
                    .checked_sub(self.offset)
                    .ok_or_else(|| invalid_position("child length changed; seek before writing"))?;
                if left == 0 {
                    self.streams[self.current].flush()?;
                    self.current += 1;
                    self.offset = 0;
                    continue;
                }
                count = count.min(usize::try_from(left).unwrap_or(usize::MAX));
            }
            self.synchronize()?;
            let written = self.streams[self.current].write(&input[..count])?;
            self.offset += written as u64;
            self.position += written as u64;
            return Ok(written);
        }
    }

    /// Flushes only the current child, matching MimeKit; an empty chain is a no-op.
    ///
    /// Explicit seeks do not flush the previous child. Flush before seeking if
    /// the child requires it. Errors propagate without moving logical position.
    fn flush(&mut self) -> io::Result<()> {
        match self.streams.get_mut(self.current) {
            Some(stream) => stream.flush(),
            None => Ok(()),
        }
    }
}

impl<R: Seek> Seek for ChainedStream<R> {
    /// Seeks within the combined current child lengths.
    ///
    /// Queries lengths in O(number of children) time. Exact boundaries select
    /// the following nonempty child, except the final end selects the last child.
    /// Unlike MimeKit's eager child rewinds, later children are positioned lazily.
    /// Seeking an empty chain to zero succeeds.
    ///
    /// Invalid offsets/length overflow return `InvalidInput`. Underlying seek
    /// errors propagate; cursors may move, but the logical position changes only
    /// on success. Retrying I/O or seeking repositions the active child. A child
    /// that reports an unexpected absolute seek result returns `InvalidData`.
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let mut lengths = Vec::with_capacity(self.streams.len());
        let mut total = 0u64;
        for stream in &mut self.streams {
            let length = stream_length(stream)?;
            total = total
                .checked_add(length)
                .ok_or_else(|| invalid_position("combined stream length overflows"))?;
            lengths.push(length);
        }
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => total.checked_add_signed(offset),
        }
        .filter(|&position| position <= total)
        .ok_or_else(|| invalid_position("seek is outside the combined stream"))?;

        let mut offset = position;
        let mut current = 0;
        for (index, &length) in lengths.iter().enumerate() {
            current = index;
            if offset < length || index + 1 == lengths.len() {
                // This is the stream which encompasses our seek offset.
                break;
            }
            offset -= length;
        }
        if let Some(stream) = self.streams.get_mut(current) {
            seek_absolute(stream, offset)?;
        }
        self.current = current;
        self.offset = offset;
        self.position = position;
        Ok(position)
    }

    /// Returns logical position without querying or moving any child.
    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}

fn invalid_position(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

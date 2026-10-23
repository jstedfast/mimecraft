// Adapted from MimeKit's IO/MemoryBlockStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom, Write};

const BLOCK_SIZE: usize = 2048;

/// A growable, seekable memory stream backed by separate 2048-byte blocks.
///
/// Instead of resizing one byte array, this stream chains non-contiguous
/// blocks. Growing the block list does not copy previously stored body bytes.
/// Unlike MimeKit's pooled buffers, blocks are owned, zero-initialized Rust
/// allocations; an empty stream allocates nothing.
///
/// Seeking is restricted to `0..=len()`, matching MimeKit, not
/// [`std::io::Cursor`]'s past-EOF seeking. Use [`set_len`](Self::set_len) to
/// extend the stream with zeros first. Resizing clamps position to the new
/// length. Writes overwrite existing bytes and can extend the end.
///
/// Lengths and positions are `u64`; storage must also fit the platform's
/// `usize`. Allocation failures from reservations return `OutOfMemory`, leaving
/// contents, length, and position unchanged (some spare blocks may be retained).
/// No global buffer pool, contiguous-buffer access, or async I/O is provided.
///
/// ```
/// use std::io::{Read, Seek, SeekFrom, Write};
/// use mimecraft::io::MemoryBlockStream;
///
/// let mut stream = MemoryBlockStream::new();
/// stream.write_all(b"body")?;
/// stream.set_len(8)?;
/// stream.seek(SeekFrom::End(-1))?;
/// stream.write_all(b"!")?;
/// stream.rewind()?;
/// let mut bytes = Vec::new();
/// stream.read_to_end(&mut bytes)?;
/// assert_eq!(bytes, b"body\0\0\0!");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug, Default)]
pub struct MemoryBlockStream {
    blocks: Vec<Vec<u8>>,
    length: usize,
    position: usize,
}

impl MemoryBlockStream {
    /// The inclusive maximum logical length, matching MimeKit's capacity limit.
    ///
    /// On platforms with a smaller `usize`, the platform limit also applies.
    /// Unlike MimeKit's exclusive write check, writes may end exactly here,
    /// consistently with resizing. Actual allocation limits may be much lower.
    pub const MAX_LEN: u64 = i32::MAX as u64 * BLOCK_SIZE as u64;

    /// Creates an empty stream at position zero without allocating.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the byte length, without moving the cursor.
    pub fn len(&self) -> u64 {
        self.length as u64
    }

    /// Returns whether the stream contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Returns the current byte offset.
    pub fn position(&self) -> u64 {
        self.position as u64
    }

    /// Resizes the stream, filling newly exposed bytes with zeros.
    ///
    /// Shrinking releases unneeded blocks and clears the retained block's tail,
    /// so later growth cannot expose truncated data. Position is clamped to the
    /// new length. An unchanged length does nothing.
    ///
    /// # Errors
    ///
    /// Returns `InvalidInput` above [`MAX_LEN`](Self::MAX_LEN) or the platform's
    /// `usize` limit, and `OutOfMemory` if a reservation fails. On error, logical
    /// length, position, and existing bytes are unchanged.
    pub fn set_len(&mut self, length: u64) -> io::Result<()> {
        let length = checked_length(length)?;
        if length > self.length {
            self.ensure_capacity(length)?;
        } else if length < self.length {
            // Shed any blocks that are no longer needed.
            self.blocks.truncate(length.div_ceil(BLOCK_SIZE));
            // Reset the retained range between the new and old lengths to zero.
            let tail = length % BLOCK_SIZE;
            if tail != 0 {
                self.blocks[length / BLOCK_SIZE][tail..].fill(0);
            }
        }
        self.position = self.position.min(length);
        self.length = length;
        Ok(())
    }

    /// Copies all logical bytes into a new contiguous vector without seeking.
    ///
    /// Spare capacity and truncated bytes are excluded. Returns `OutOfMemory`
    /// if the contiguous allocation cannot be reserved, even if the segmented
    /// stream itself fits in memory.
    pub fn to_vec(&self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.length)
            .map_err(allocation_error)?;
        for block in &self.blocks {
            let count = (self.length - bytes.len()).min(BLOCK_SIZE);
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&block[..count]);
        }
        Ok(bytes)
    }

    fn ensure_capacity(&mut self, length: usize) -> io::Result<()> {
        let required = length.div_ceil(BLOCK_SIZE);
        if required <= self.blocks.len() {
            return Ok(());
        }
        self.blocks
            .try_reserve(required - self.blocks.len())
            .map_err(allocation_error)?;
        while self.blocks.len() < required {
            let mut block = Vec::new();
            block
                .try_reserve_exact(BLOCK_SIZE)
                .map_err(allocation_error)?;
            // Initialized blocks also keep resize-created gaps deterministic.
            block.resize(BLOCK_SIZE, 0);
            self.blocks.push(block);
        }
        Ok(())
    }
}

impl Read for MemoryBlockStream {
    /// Reads across blocks, filling the output up to the logical end.
    ///
    /// Empty reads and reads at EOF return zero without moving position.
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.length - self.position);
        let mut copied = 0;
        while copied < count {
            let offset = self.position % BLOCK_SIZE;
            let n = (BLOCK_SIZE - offset).min(count - copied);
            output[copied..copied + n]
                .copy_from_slice(&self.blocks[self.position / BLOCK_SIZE][offset..offset + n]);
            self.position += n;
            copied += n;
        }
        Ok(count)
    }
}

impl Write for MemoryBlockStream {
    /// Writes the complete input, allocating additional blocks as needed.
    ///
    /// Empty writes do nothing. Capacity is reserved before copying bytes, so
    /// an allocation or length error cannot hide partial progress.
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        let end = self
            .position()
            .checked_add(input.len() as u64)
            .ok_or_else(|| invalid_length("write length overflows"))?;
        let end = checked_length(end)?;
        self.ensure_capacity(end)?;
        let mut copied = 0;
        while copied < input.len() {
            let offset = self.position % BLOCK_SIZE;
            let n = (BLOCK_SIZE - offset).min(input.len() - copied);
            self.blocks[self.position / BLOCK_SIZE][offset..offset + n]
                .copy_from_slice(&input[copied..copied + n]);
            self.position += n;
            copied += n;
        }
        self.length = self.length.max(end);
        Ok(input.len())
    }

    /// Does nothing: all written data is already in memory.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for MemoryBlockStream {
    /// Seeks within `0..=len()`, with checked signed-offset arithmetic.
    ///
    /// Before-start, past-end, and overflowing seeks return `InvalidInput`
    /// without changing position. Seeking never allocates or changes length.
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position().checked_add_signed(offset),
            SeekFrom::End(offset) => self.len().checked_add_signed(offset),
        };
        // Sanity-check the resultant offset before changing the cursor.
        let position = position
            .filter(|&position| position <= self.len())
            .ok_or_else(|| invalid_length("seek is outside the memory stream"))?;
        self.position = position as usize;
        Ok(position)
    }

    /// Returns the current position without moving it.
    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position())
    }
}

fn checked_length(length: u64) -> io::Result<usize> {
    if length > MemoryBlockStream::MAX_LEN {
        return Err(invalid_length("memory stream exceeds its maximum length"));
    }
    usize::try_from(length).map_err(|_| invalid_length("length exceeds platform address space"))
}

fn invalid_length(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn allocation_error(error: std::collections::TryReserveError) -> io::Error {
    io::Error::new(io::ErrorKind::OutOfMemory, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_does_not_relocate_existing_block_data() {
        let mut stream = MemoryBlockStream::new();
        assert!(stream.blocks.is_empty());
        stream.write_all(b"body").unwrap();
        let original = stream.blocks[0].as_ptr();
        stream.write_all(&vec![1; BLOCK_SIZE * 32]).unwrap();
        assert_eq!(stream.blocks[0].as_ptr(), original);
        stream.set_len(4).unwrap();
        assert_eq!(stream.blocks.len(), 1);
        assert_eq!(stream.blocks[0].as_ptr(), original);
        assert!(stream.blocks[0][4..].iter().all(|&byte| byte == 0));
        stream.set_len(0).unwrap();
        assert!(stream.blocks.is_empty());
    }

    #[test]
    fn length_limit_is_checked_without_allocating() {
        let maximum = MemoryBlockStream::MAX_LEN.min(usize::MAX as u64);
        assert_eq!(checked_length(maximum).unwrap() as u64, maximum);
        assert_eq!(
            checked_length(maximum + 1).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

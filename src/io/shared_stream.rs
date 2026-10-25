// Adapted from MimeKit's MimeParser.Initialize and IO/BoundedStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
};

/// A shared, read-only seekable source with a separate cursor for each clone.
///
/// Owns `R`, which may itself be a mutable borrow. The source is dropped when
/// the last handle drops. A borrowed source remains owned by its caller.
/// Seeking and reading are synchronized together, so interleaved readers do
/// not disturb one another. No source-mutation API is exposed.
///
/// The underlying data must remain unchanged while ranges refer to it.
/// Source errors propagate without advancing the handle's logical cursor.
/// A poisoned lock returns an I/O error rather than reusing uncertain state.
#[derive(Debug)]
pub struct SharedStream<R> {
    inner: Arc<Mutex<R>>,
    position: u64,
}

impl<R: Seek> SharedStream<R> {
    /// Takes ownership of a source at its current position.
    ///
    /// Queries the source position; errors propagate without constructing a handle.
    pub fn new(mut source: R) -> io::Result<Self> {
        let position = source.stream_position()?;
        Ok(Self {
            inner: Arc::new(Mutex::new(source)),
            position,
        })
    }
}

impl<R> SharedStream<R> {
    /// Returns this handle's absolute cursor without I/O.
    pub fn position(&self) -> u64 {
        self.position
    }

    fn lock(&self) -> io::Result<std::sync::MutexGuard<'_, R>> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("shared source lock poisoned"))
    }
}

impl<R> Clone for SharedStream<R> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            position: self.position,
        }
    }
}

impl<R: Read + Seek> Read for SharedStream<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let count = (output.len() as u64).min(u64::MAX - self.position) as usize;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source position overflow",
            ));
        }
        let read = {
            let mut source = self.lock()?;
            // Reading a previously parsed part moves the shared stream.
            // Restore this handle's position before reading, just as MimeKit's
            // Initialize restores the parser position. Keep seek + read atomic.
            if source.stream_position()? != self.position {
                super::seek_absolute(&mut *source, self.position)?;
            }
            source.read(&mut output[..count])?
        };
        self.position += read as u64;
        Ok(read)
    }
}

impl<R: Seek> Seek for SharedStream<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = {
            let mut source = self.lock()?;
            let target = match from {
                SeekFrom::Start(offset) => Some(offset),
                SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
                SeekFrom::End(offset) => source.seek(SeekFrom::End(0))?.checked_add_signed(offset),
            }
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid shared source seek")
            })?;
            super::seek_absolute(&mut *source, target)?;
            target
        };
        self.position = target;
        Ok(target)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}

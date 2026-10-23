// Read-side behavior adapted from MimeKit's IO/FilteredStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, Read};

use crate::filters::MimeFilter;

const READ_BUFFER_SIZE: usize = 4096;

/// A reader that passes source bytes through one incremental MIME filter.
///
/// Implements [`Read`] and [`BufRead`], retaining filtered output between calls.
/// Source reads use a 4096-byte buffer. A filter may consume several chunks
/// before producing output; this does not cause a premature end-of-stream.
///
/// The first source read returning zero permanently ends input and calls
/// [`MimeFilter::finish`] exactly once. Its output is delivered before this
/// reader reports EOF. Errors, including `Interrupted` and `WouldBlock`, are
/// propagated without finishing or resetting the filter; callers can retry.
/// Reads into an empty destination perform no I/O and do not finish the filter.
///
/// The source and filter are owned, but either may be a mutable reference.
/// Dropping the adapter does not read, finish, or reset anything. Pass
/// `&mut source` to keep using a source after dropping the adapter, taking care
/// not to lose read-ahead output or filter state. [`into_parts`](Self::into_parts)
/// recovers the source, filter, and unread filtered output together.
///
/// Nest readers to compose filters. All output, including an upstream filter's
/// final bytes, reaches downstream filters before they are finished.
/// This is the read-side adaptation of MimeKit's `FilteredStream`, not a
/// seekable or bidirectional stream.
///
/// ```
/// use std::io::Read;
/// use mimecraft::{
///     encodings::Base64Decoder,
///     filters::DecoderFilter,
///     io::FilteredReader,
/// };
///
/// let filter = DecoderFilter::new(Base64Decoder::new());
/// let mut reader = FilteredReader::new(&b"aGVsbG8=\r\n"[..], filter);
/// let mut decoded = Vec::new();
/// reader.read_to_end(&mut decoded)?;
/// assert_eq!(decoded, b"hello");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct FilteredReader<R, F> {
    inner: R,
    filter: F,
    input: [u8; READ_BUFFER_SIZE],
    output: Vec<u8>,
    position: usize,
    eof: bool,
}

impl<R: Read, F: MimeFilter> FilteredReader<R, F> {
    /// Wraps a source and filter without resetting their existing state.
    pub fn new(inner: R, filter: F) -> Self {
        Self {
            inner,
            filter,
            input: [0; READ_BUFFER_SIZE],
            output: Vec::new(),
            position: 0,
            eof: false,
        }
    }

    /// Returns the underlying source, which may be ahead of delivered output.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Returns mutable access to the source.
    ///
    /// Reading or seeking it directly bypasses filtering and can skip or reorder
    /// data. Changing the source does not clear this adapter's EOF state.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Returns the filter, for example to inspect an analyzer or codec checksum.
    ///
    /// Its state includes all read-ahead input, not just bytes already delivered.
    pub fn filter(&self) -> &F {
        &self.filter
    }

    /// Returns unread filtered output without performing I/O.
    ///
    /// An empty slice does not imply EOF; [`BufRead::fill_buf`] may obtain more.
    pub fn buffer(&self) -> &[u8] {
        &self.output[self.position..]
    }

    /// Returns the source, filter, and unread filtered output, in that order.
    ///
    /// Does not read, finish, or reset. The returned output must be consumed
    /// before any further filtered source bytes to preserve ordering. The
    /// filter may also retain pending state, so returning only the source
    /// would not be sufficient to resume a partially processed stream.
    ///
    /// This does not preserve the adapter's EOF latch: a new reader wrapping
    /// these components will finish the filter again when it reaches EOF.
    pub fn into_parts(mut self) -> (R, F, Vec<u8>) {
        drop(self.output.drain(..self.position));
        (self.inner, self.filter, self.output)
    }
}

impl<R: Read, F: MimeFilter> Read for FilteredReader<R, F> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }

        let available = self.fill_buf()?;
        let count = output.len().min(available.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl<R: Read, F: MimeFilter> BufRead for FilteredReader<R, F> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        // Filters may consume input without producing output (for example, a
        // trailing CR). Keep reading until output or source EOF, not merely
        // until the first successful source read.
        while self.position == self.output.len() {
            if self.eof {
                return Ok(&[]);
            }

            self.output.clear();
            self.position = 0;
            let count = self.inner.read(&mut self.input)?;
            if count == 0 {
                // At source EOF, deliver any data buffered by the filter.
                // Latch EOF so repeated reads never finish the filter again.
                self.eof = true;
                self.filter.finish(&mut self.output);
            } else {
                self.filter.filter(&self.input[..count], &mut self.output);
            }
        }

        Ok(self.buffer())
    }

    fn consume(&mut self, amount: usize) {
        self.position += amount.min(self.output.len() - self.position);
    }
}

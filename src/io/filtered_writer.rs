// Write-side behavior adapted from MimeKit's IO/FilteredStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Write};

use crate::filters::MimeFilter;

const INPUT_CHUNK_SIZE: usize = 4096;

/// A buffered writer that passes input through one incremental MIME filter.
///
/// Each nonempty [`write`](Write::write) first drains previously produced
/// output, then accepts at most 4096 input bytes and buffers their filtered
/// output. Success reports input accepted, not bytes already delivered to the
/// sink. Once input is accepted, that call performs no further I/O, so an I/O
/// error never causes callers to unknowingly submit the same input twice.
///
/// [`flush`](Write::flush) drains produced output and flushes the sink, but
/// does not finish the filter or force out incomplete codec groups. Call
/// [`finish`](Self::finish) explicitly to end input. This deliberately separates
/// Rust's I/O flushing from MimeKit's filter-finalizing stream `Flush`.
///
/// Short writes and failed drains retain the unwritten suffix. Interrupted
/// writes are retried; other errors propagate. Empty writes are no-ops.
/// Neither dropping nor dismantling this adapter flushes or finishes it.
/// Owned components are dropped normally; use mutable references to retain them.
///
/// Nested writers process input from the outermost filter inward. Finish the
/// outer writer successfully, then each inner writer in turn: flushing an inner
/// writer does not finalize its filter.
///
/// ```
/// use std::io::Write;
/// use mimecraft::{
///     encodings::Base64Encoder,
///     filters::EncoderFilter,
///     io::FilteredWriter,
/// };
///
/// let mut writer = FilteredWriter::new(Vec::new(), EncoderFilter::new(Base64Encoder::new()));
/// writer.write_all(b"C")?;
/// writer.flush()?; // does not pad or end the incomplete Base64 group
/// writer.write_all(b"at")?;
/// writer.finish()?;
/// assert_eq!(writer.get_ref(), b"Q2F0\r\n");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
#[must_use = "call finish() to finalize the filter and flush its output"]
pub struct FilteredWriter<W, F> {
    inner: W,
    filter: F,
    output: Vec<u8>,
    position: usize,
    finished: bool,
}

impl<W: Write, F: MimeFilter> FilteredWriter<W, F> {
    /// Wraps a sink and filter without resetting their existing state.
    pub fn new(inner: W, filter: F) -> Self {
        Self {
            inner,
            filter,
            output: Vec::new(),
            position: 0,
            finished: false,
        }
    }

    /// Returns the underlying sink, which may not have received buffered output.
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Returns mutable access to the sink.
    ///
    /// Writing directly bypasses this filter and can reorder buffered output.
    /// For nested writers, finish this writer successfully before finishing
    /// its inner writer through this reference.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    /// Returns the filter, whose state includes all accepted input.
    pub fn filter(&self) -> &F {
        &self.filter
    }

    /// Returns filtered bytes not yet accepted by the underlying sink.
    ///
    /// Does not include data still pending inside the filter, such as an
    /// incomplete Base64 group, or data buffered internally by the sink.
    pub fn buffer(&self) -> &[u8] {
        &self.output[self.position..]
    }

    /// Ends input, finishes the filter once, drains output, and flushes the sink.
    ///
    /// Further nonempty writes fail with [`io::ErrorKind::BrokenPipe`], even if
    /// this call fails during I/O. Retry `finish` to resume delivery or retry
    /// the sink flush without producing the filter's final bytes again.
    /// Repeated successful calls only flush the sink again.
    ///
    /// # Errors
    ///
    /// Propagates sink errors, retaining any unwritten bytes. A zero-byte sink
    /// write with output pending returns [`io::ErrorKind::WriteZero`].
    pub fn finish(&mut self) -> io::Result<()> {
        if !self.finished {
            // As in MimeKit, latch finalization before attempting delivery, so
            // retrying failed I/O cannot duplicate terminators or final groups.
            self.finished = true;
            self.filter.finish(&mut self.output);
        }

        self.flush()
    }

    /// Returns the sink, filter, and unwritten filtered output, in that order.
    ///
    /// Does not flush, finish, or reset. Deliver the returned output directly
    /// to the sink before any subsequently filtered bytes; passing it through
    /// the filter again would transform it twice. The filter itself may retain
    /// incomplete input that has not produced output yet.
    ///
    /// This does not preserve the finalization latch. Rewrapping a previously
    /// finished filter in a new writer and finishing it may emit another tail.
    pub fn into_parts(mut self) -> (W, F, Vec<u8>) {
        drop(self.output.drain(..self.position));
        (self.inner, self.filter, self.output)
    }

    fn drain_output(&mut self) -> io::Result<()> {
        while self.position < self.output.len() {
            match self.inner.write(&self.output[self.position..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write buffered filter output",
                    ));
                }
                Ok(count) => self.position += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        self.output.clear();
        self.position = 0;
        Ok(())
    }
}

impl<W: Write, F: MimeFilter> Write for FilteredWriter<W, F> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        if self.finished {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "cannot write after filter finalization",
            ));
        }

        // Fallible I/O must precede consuming new input. After filtering, the
        // codec state has advanced and Write must report those bytes accepted.
        self.drain_output()?;
        let count = input.len().min(INPUT_CHUNK_SIZE);
        self.filter.filter(&input[..count], &mut self.output);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain_output()?;
        self.inner.flush()
    }
}

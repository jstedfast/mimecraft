// Ported from MimeKit's Tnef/TnefPropertyWriter.cs embedded-message writing.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    fmt,
    io::{self, Write},
    ops::{Deref, DerefMut},
};

use super::TnefWriter;

pub(super) trait EmbeddedOutput {
    fn write_embedded(&mut self, offset: u64, bytes: &[u8]) -> io::Result<usize>;
    fn flush_embedded(&mut self, offset: u64) -> io::Result<()>;
    fn finish_embedded(&mut self, offset: u64) -> io::Result<()>;
}

/// The append-only, borrowed sink of an embedded TNEF writer.
///
/// Erasing the enclosing writer/scratch types permits runtime recursion without
/// infinitely nested generic types. No boxed sink or extra payload buffer is used.
/// Only the owning [`TnefEmbeddedWriter`] can complete the enclosing Object value.
pub struct TnefEmbeddedSink<'a> {
    output: &'a mut dyn EmbeddedOutput,
    offset: u64,
}

impl fmt::Debug for TnefEmbeddedSink<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TnefEmbeddedSink")
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl<'a> TnefEmbeddedSink<'a> {
    pub(super) fn new(output: &'a mut dyn EmbeddedOutput, offset: u64) -> Self {
        Self { output, offset }
    }
}

impl Write for TnefEmbeddedSink<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.write_embedded(self.offset, bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush_embedded(self.offset)
    }
}

/// An exclusively borrowed writer for one embedded TNEF Object value.
///
/// Obtain this from [`super::TnefPropertyWriter::open_embedded_message`].
/// The IMessage IID is written automatically, the legacy key is zero, and the
/// codepage is inherited unless explicitly selected. Named IDs are local to each
/// message. Writer operations are available through [`Deref`] and [`DerefMut`].
/// Nested buffered attributes use memory blocks by default; their `_with_buffer`
/// methods accept independent caller-owned scratch, including borrowed files.
///
/// `flush` flushes enclosing scratch, not outer output. Consuming `finish`
/// completes the embedded header/attributes, flushes scratch, and patches the
/// enclosing Object length/padding/count. The outer property writer and message
/// still need completion. Drop performs no I/O; dropped, forgotten, or failed
/// children leave the enclosing property writer blocked.
///
/// ```
/// use mimecraft::tnef::{TnefAttributeTag as A, TnefPropertyTag as P, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0)?;
/// writer.write_attribute(A::ATTACH_RENDER_DATA, &[0; 14])?;
/// let mut properties = writer.open_properties(A::ATTACHMENT)?;
/// properties.write_tag(P::ATTACH_DATA_OBJ)?;
/// let mut embedded = properties.open_embedded_message()?;
/// embedded.write_string(A::SUBJECT, "inner")?;
/// embedded.finish()?;
/// properties.finish()?;
/// writer.finish()?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// Writing to the outer property writer is blocked until the message is complete:
///
/// ```compile_fail
/// use mimecraft::tnef::{TnefAttributeTag as A, TnefPropertyTag as P, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
/// let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
/// properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
/// let embedded = properties.open_embedded_message().unwrap();
/// properties.write_tag(P::SUBJECT_W).unwrap();
/// embedded.finish().unwrap();
/// ```
#[derive(Debug)]
#[must_use = "call finish() or the property writer remains faulted"]
pub struct TnefEmbeddedWriter<'a> {
    writer: TnefWriter<TnefEmbeddedSink<'a>>,
}

impl<'a> TnefEmbeddedWriter<'a> {
    pub(super) fn new(sink: TnefEmbeddedSink<'a>, codepage: u32) -> io::Result<Self> {
        Ok(Self {
            writer: TnefWriter::new(sink, codepage, 0)?,
        })
    }

    /// Completes the embedded message and then its enclosing Object value.
    ///
    /// # Errors
    ///
    /// Rejects unfinished nested attributes/values. Propagates scratch I/O and
    /// size errors immediately rather than deferring disposal-callback failures.
    /// Any failure leaves the enclosing property writer blocked.
    pub fn finish(mut self) -> io::Result<()> {
        self.writer.finish()?;
        let sink = self.writer.into_inner();
        sink.output.finish_embedded(sink.offset)
    }
}

impl<'a> Deref for TnefEmbeddedWriter<'a> {
    type Target = TnefWriter<TnefEmbeddedSink<'a>>;

    fn deref(&self) -> &Self::Target {
        &self.writer
    }
}

impl DerefMut for TnefEmbeddedWriter<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.writer
    }
}

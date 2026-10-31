// Adapted from MimeKit's TnefReader.cs and TnefPropertyReader.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, Read},
    ops::Deref,
};

use super::{Logger, TnefDateTime, TnefPropertyReader, TnefReadError, TnefReader, TnefValueReader};

pub(super) trait EmbeddedInput {
    fn read_embedded(&mut self, buffer: &mut [u8]) -> io::Result<usize>;
}

/// Bounded source of a borrowed embedded reader. Constructed only by
/// [`TnefPropertyReader::open_embedded_message`].
///
/// Erasing the parent's source type keeps recursive traversal from generating
/// an infinitely nested generic type. No payload buffer or boxed source is used.
pub struct TnefEmbeddedSource<'a> {
    input: &'a mut dyn EmbeddedInput,
}

impl<'a> TnefEmbeddedSource<'a> {
    pub(super) fn new(input: &'a mut dyn EmbeddedInput) -> Self {
        Self { input }
    }
}

impl Read for TnefEmbeddedSource<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.input.read_embedded(buffer)
    }
}

/// An exclusively borrowed embedded TNEF message with inherited diagnostics.
///
/// Uses the parent's current charset, copied options, and shared byte/report
/// budgets. Source/logger failures fault the entire borrowed reader tree.
/// Offsets are relative to the outermost stream. Opening and dropping perform
/// no I/O; the IID prefix is skipped on first advance. Dropping does not verify
/// pending checksums. The parent resumes by skipping any unread value/padding.
///
/// Immutable metadata/status accessors are available through [`Deref`].
/// Mutating operations below always use the inherited logger.
///
/// ```compile_fail
/// use mimecraft::tnef::TnefPropertyReader;
/// fn advance_while_borrowed<R: std::io::Read>(properties: &mut TnefPropertyReader<'_, R>) {
///     let mut child = properties.open_embedded_message().unwrap();
///     properties.read_next_property().unwrap();
///     child.read_next().unwrap();
/// }
/// ```
pub struct TnefEmbeddedReader<'p, 'r> {
    pub(super) reader: TnefReader<TnefEmbeddedSource<'p>>,
    pub(super) logger: Logger<'r>,
}

impl<'p> Deref for TnefEmbeddedReader<'p, '_> {
    type Target = TnefReader<TnefEmbeddedSource<'p>>;

    fn deref(&self) -> &Self::Target {
        &self.reader
    }
}

impl<'p> TnefEmbeddedReader<'p, '_> {
    /// Advances and verifies the previous attribute, using inherited diagnostics.
    ///
    /// # Errors
    ///
    /// Source/logger/arithmetic errors fault the reader tree.
    pub fn read_next(&mut self) -> Result<bool, TnefReadError> {
        self.reader.run(|reader| reader.next_core(&mut self.logger))
    }

    /// Opens the current property-bearing attribute with inherited diagnostics.
    ///
    /// # Errors
    ///
    /// Requires an unclaimed property-bearing attribute and a nonfaulted tree.
    pub fn open_properties(
        &mut self,
    ) -> Result<TnefPropertyReader<'_, TnefEmbeddedSource<'p>>, TnefReadError> {
        if self.logger.is_some() {
            self.reader.open_properties_with_logger(&mut self.logger)
        } else {
            self.reader.open_properties()
        }
    }

    /// Opens a bounded raw attribute stream with inherited diagnostics.
    ///
    /// # Errors
    ///
    /// Requires an unclaimed attribute and a nonfaulted tree.
    pub fn open_value(
        &mut self,
    ) -> Result<TnefValueReader<'_, TnefEmbeddedSource<'p>>, TnefReadError> {
        if self.logger.is_some() {
            self.reader.open_value_with_logger(&mut self.logger)
        } else {
            self.reader.open_value()
        }
    }

    /// Collects raw bytes under the shared owned-byte budget.
    ///
    /// # Errors
    ///
    /// Same as [`TnefReader::read_value_bytes`], with inherited diagnostics.
    pub fn read_value_bytes(&mut self) -> Result<Vec<u8>, TnefReadError> {
        self.reader.bytes_core(&mut self.logger)
    }

    /// Collects and decodes an attribute string using this message's charset.
    ///
    /// # Errors
    ///
    /// Same as [`TnefReader::read_value_string`], with inherited diagnostics.
    pub fn read_value_string(&mut self) -> Result<String, TnefReadError> {
        self.reader.string_core(&mut self.logger)
    }

    /// Reads a repeatable integer attribute.
    ///
    /// # Errors
    ///
    /// Same as [`TnefReader::read_value_i32`], with inherited diagnostics.
    pub fn read_value_i32(&mut self) -> Result<i32, TnefReadError> {
        self.reader.scalar_core(&mut self.logger)
    }

    /// Reads a checked, repeatable date attribute.
    ///
    /// # Errors
    ///
    /// Same as [`TnefReader::read_value_datetime`], with inherited diagnostics.
    pub fn read_value_datetime(&mut self) -> Result<TnefDateTime, TnefReadError> {
        self.reader.date_core(&mut self.logger)
    }
}

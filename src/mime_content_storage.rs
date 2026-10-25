// Adapted from MimeKit's MimeContentStorage.cs and MimeParser.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use crate::io::{BoundedStream, SharedStream};

/// Where a parser stores encoded MIME part bodies.
///
/// Headers, multipart preambles/epilogues, and tree structure are retained in
/// memory in both modes. No mode decodes content during parsing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MimeContentStorage {
    /// Copy bodies into memory so messages no longer depend on the source.
    #[default]
    Memory,
    /// Reference ranges in the seekable source instead of copying bodies.
    ///
    /// The source must remain unchanged. Owned sources stay alive until the
    /// parser and all source-backed bodies drop; borrowed sources are protected
    /// by Rust lifetimes. Parsing still scans every byte and enforces limits.
    Source,
}

/// The body stream returned by [`crate::MimeParser::with_storage`].
///
/// Both variants read and seek relative to the body, starting at zero.
/// Source-backed ranges are read-only and retain shared ownership of the input.
/// Use `Memory` to supply replacement body bytes without modifying that input.
#[derive(Debug)]
pub enum MimeContentStream<R> {
    /// An owned copy of the encoded body.
    Memory(Cursor<Vec<u8>>),
    /// A bounded view of the encoded body in the original source.
    Source(BoundedStream<SharedStream<R>>),
}

impl<R: Read + Seek> Read for MimeContentStream<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Memory(source) => source.read(output),
            Self::Source(source) => source.read(output),
        }
    }
}

impl<R: Seek> Seek for MimeContentStream<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Memory(source) => source.seek(from),
            Self::Source(source) => source.seek(from),
        }
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        match self {
            Self::Memory(source) => source.stream_position(),
            Self::Source(source) => source.stream_position(),
        }
    }
}

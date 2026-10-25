// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Synchronous stream ranges and adapters for buffer-oriented [`crate::filters`].
//!
//! [`BoundedStream`] provides relative seeking and range-limited reading/writing
//! over a seekable source, without copying MIME body bytes.
//! [`SharedStream`] shares read-only source access through independent cursors.
//! [`ChainedStream`] concatenates seekable sources without combining their data.
//! [`MemoryBlockStream`] stores growable body data in separate memory blocks,
//! with seeking and zero-initialized resizing.
//! [`MeasuringStream`] measures written output without storing any body bytes.
//! [`FilteredReader`] implements [`std::io::Read`] and [`std::io::BufRead`].
//! Nest readers to apply several filters in order, including their final output.
//! [`FilteredWriter`] implements [`std::io::Write`], with non-finalizing flushing
//! and explicit [`finish`](FilteredWriter::finish). Finish nested writers from
//! the outside inward. Filtered adapters do not support seeking.
//! Asynchronous I/O is not implemented yet.

mod bounded_stream;
mod chained_stream;
mod filtered_reader;
mod filtered_writer;
mod measuring_stream;
mod memory_block_stream;
mod shared_stream;

pub use bounded_stream::BoundedStream;
pub use chained_stream::ChainedStream;
pub use filtered_reader::FilteredReader;
pub use filtered_writer::FilteredWriter;
pub use measuring_stream::MeasuringStream;
pub use memory_block_stream::MemoryBlockStream;
pub use shared_stream::SharedStream;

use std::io::{self, Seek, SeekFrom};

fn seek_absolute<S: Seek>(stream: &mut S, target: u64) -> io::Result<()> {
    let actual = stream.seek(SeekFrom::Start(target))?;
    if actual != target {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "underlying stream did not seek to the requested offset",
        ));
    }
    Ok(())
}

fn stream_length<S: Seek>(stream: &mut S) -> io::Result<u64> {
    let original = stream.stream_position()?;
    let end = stream.seek(SeekFrom::End(0))?;
    if end != original {
        seek_absolute(stream, original)?;
    }
    Ok(end)
}

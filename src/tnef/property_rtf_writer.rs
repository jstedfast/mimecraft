// Ported from MimeKit's Tnef/TnefPropertyWriter.cs compressed-RTF value integration.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom, Write};

use super::{RtfCompressedWriter, RtfCompressionMode, TnefPropertyValueWriter};
use crate::io::MemoryBlockStream;

/// Encodes one Binary property value as MS-OXRTFCP LZFu or MELA RTF.
///
/// Obtain this child from [`super::TnefPropertyWriter::open_rtf_compressed`].
/// The existing encoder writes directly to attribute scratch, with its dictionary
/// and input buffers inline. It does not allocate a second property buffer.
/// RTF syntax is not validated, and input bytes are not charset-converted.
///
/// `flush` encodes available input and flushes scratch while preserving encoder
/// lookahead; it does not finish the value or emit the attribute. Explicit
/// `finish` patches the RTF header, flushes scratch, then completes the Binary
/// length/padding/count. The property writer and its parent still need finishing.
/// No I/O occurs on drop. Dropped or forgotten children leave properties blocked.
///
/// ```
/// use std::io::Write;
/// use mimecraft::tnef::{RtfCompressionMode, TnefAttributeTag as A,
///                      TnefPropertyTag as P, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0)?;
/// let mut properties = writer.open_properties(A::MAPI_PROPERTIES)?;
/// properties.write_tag(P::RTF_COMPRESSED)?;
/// let mut rtf = properties.open_rtf_compressed(RtfCompressionMode::Compressed)?;
/// rtf.write_all(b"{\\rtf1 hello hello hello}")?;
/// rtf.finish()?;
/// properties.finish()?;
/// writer.finish()?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// The property cannot advance while the encoder borrows it:
///
/// ```compile_fail
/// use mimecraft::tnef::{RtfCompressionMode, TnefAttributeTag as A,
///                      TnefPropertyTag as P, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
/// let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
/// properties.write_tag(P::RTF_COMPRESSED).unwrap();
/// let rtf = properties.open_rtf_compressed(RtfCompressionMode::Compressed).unwrap();
/// properties.finish().unwrap();
/// rtf.finish().unwrap();
/// ```
#[derive(Debug)]
#[must_use = "call finish() or the property writer remains faulted"]
pub struct TnefRtfPropertyWriter<'p, 'a, W, B = MemoryBlockStream> {
    inner: RtfCompressedWriter<ValueSink<'p, 'a, W, B>>,
}

impl<'p, 'a, W: Write, B: Read + Write + Seek> TnefRtfPropertyWriter<'p, 'a, W, B> {
    pub(super) fn new(
        value: TnefPropertyValueWriter<'p, 'a, W, B>,
        mode: RtfCompressionMode,
    ) -> io::Result<Self> {
        Ok(Self {
            inner: RtfCompressedWriter::new(ValueSink { value, position: 0 }, mode)?,
        })
    }

    /// Compression mode selected when opening this value.
    pub fn compression_mode(&self) -> RtfCompressionMode {
        self.inner.compression_mode()
    }

    /// Number of raw input bytes acknowledged by successful writes.
    pub fn raw_size(&self) -> u64 {
        self.inner.raw_size()
    }

    /// Whether failed encoding or scratch I/O prevents further operations.
    pub fn is_faulted(&self) -> bool {
        self.inner.is_faulted()
    }

    /// Finalizes the RTF stream and then completes its Binary property value.
    ///
    /// # Errors
    ///
    /// Propagates encoding, size, and scratch write/seek/flush errors. Any failure
    /// leaves the property writer blocked. Use property `into_inner` to recover
    /// scratch without finalizing; owned scratch retains its normal drop behavior.
    pub fn finish(mut self) -> io::Result<()> {
        // Complete the encoded stream before patching its enclosing value length.
        // Explicit finish reports errors immediately instead of a disposal callback.
        self.inner.finish()?;
        self.inner.into_inner().value.finish()
    }
}

impl<W: Write, B: Read + Write + Seek> Write for TnefRtfPropertyWriter<'_, '_, W, B> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.inner.write(input)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

// Only the encoder can seek within its own value. Logical seeks never change
// the append cursor: overwrites patch existing bytes and restore the scratch end.
// Neither the public raw-value stream nor the RTF child exposes Seek.
#[derive(Debug)]
struct ValueSink<'p, 'a, W, B> {
    value: TnefPropertyValueWriter<'p, 'a, W, B>,
    position: u64,
}

impl<W: Write, B: Read + Write + Seek> Write for ValueSink<'_, '_, W, B> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let count = if self.position == self.value.len() {
            self.value.write(input)?
        } else {
            self.value.patch(self.position, input)?;
            input.len()
        };
        self.position += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.value.flush()
    }
}

impl<W: Write, B: Read + Write + Seek> Seek for ValueSink<'_, '_, W, B> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        if self.value.is_faulted() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the RTF value sink is faulted",
            ));
        }
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.value.len().checked_add_signed(offset),
        }
        .filter(|&position| position <= self.value.len())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot seek outside the RTF property value",
            )
        })?;
        self.position = position;
        Ok(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tnef::{TnefAttributeTag as A, TnefPropertyTag as P, TnefValue as V, TnefWriter};

    #[test]
    fn private_encoder_seeks_and_patches_are_bounded_to_existing_value_bytes() {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
        properties.write_tag(P::IMPORTANCE).unwrap();
        properties.write_value(&V::I32(42)).unwrap();
        properties.write_tag(P::ATTACH_DATA_BIN).unwrap();
        let mut sink = ValueSink {
            value: properties.open_value().unwrap(),
            position: 0,
        };
        sink.write_all(b"abcdefgh").unwrap();
        assert_eq!(sink.stream_position().unwrap(), 8);
        for from in [
            SeekFrom::Start(u64::MAX),
            SeekFrom::Current(i64::MIN),
            SeekFrom::Current(i64::MAX),
            SeekFrom::End(-9),
            SeekFrom::End(1),
        ] {
            assert_eq!(
                sink.seek(from).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(sink.stream_position().unwrap(), 8);
        }
        sink.seek(SeekFrom::End(-6)).unwrap();
        assert_eq!(
            sink.write(&[0; 7]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(sink.stream_position().unwrap(), 2);
        assert_eq!(
            sink.value.patch(u64::MAX, &[0]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!sink.value.is_faulted());
        sink.rewind().unwrap();
        sink.write_all(b"XYZ").unwrap();
        assert_eq!(sink.value.len(), 8, "overwrites must not grow the value");
        sink.seek(SeekFrom::End(0)).unwrap();
        sink.write_all(b"ij").unwrap();
        sink.value.finish().unwrap();
        let actual = properties.finish().unwrap().to_vec().unwrap();
        let mut expected: Vec<u8> = [2u32, P::IMPORTANCE.0, 42, P::ATTACH_DATA_BIN.0, 1, 10]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        expected.extend_from_slice(b"XYZdefghij\0\0");
        assert_eq!(actual, expected);
        writer.finish().unwrap();
    }
}

// Ported from MimeKit's Tnef/RtfCompressedWriteStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Seek, SeekFrom, Write};

use super::{RtfCompressionMode, rtf_compressed::INITIALIZER};
use crate::crc32;

const HEADER_SIZE: usize = 16;
const DICTIONARY_SIZE: usize = 4096;
const DICTIONARY_MASK: usize = DICTIONARY_SIZE - 1;
const MIN_MATCH: usize = 2;
const MAX_MATCH: usize = 17;
const MAX_CHAIN: usize = 64;
const MAX_RAW_SIZE: u64 = i32::MAX as u64;
const MAX_CONTENT_SIZE: u64 = MAX_RAW_SIZE - 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Open,
    Finished,
    Faulted,
}

/// Encodes [MS-OXRTFCP] compressed (LZFu) or uncompressed (MELA) RTF.
///
/// Construction reserves a sixteen-byte header at the sink's current position.
/// [`finish`](Self::finish) writes the terminator, patches sizes/mode/CRC, restores
/// the end position, and flushes the sink. It must be called explicitly: dropping
/// or dismantling the writer never performs I/O. Use `&mut sink` to retain it.
/// An owned sink still has its normal drop behavior.
/// Existing bytes outside the written range are not truncated or removed.
///
/// Compressed writes accept at most one 4 KiB input buffer at a time. Encoding
/// previously accepted input may perform I/O before accepting new bytes, never
/// after: a successful write reports exactly the input accepted. [`flush`](Write::flush)
/// encodes available input and flushes complete token groups, but preserves up
/// to seventeen bytes of lookahead and the incomplete group. It does not finalize
/// the header.
/// Chunk boundaries and intermediate flushes do not change the encoded bytes.
///
/// Short writes and interrupted I/O are handled. Other sink failures permanently
/// fault the writer, since a partly written header/group cannot safely be retried.
/// Such failures leave provisional output; subsequent operations return
/// [`io::ErrorKind::BrokenPipe`]. Input-size preflight errors leave it usable.
/// As in MimeKit, RAWSIZE is limited to `i32::MAX` and CONTENTS to `i32::MAX - 12`.
///
/// The dictionary, hash chains, input, and token group are fixed inline arrays.
/// Successful encoding needs no heap allocation beyond the sink's own storage.
/// The writer does not validate RTF syntax, choose compression automatically,
/// or write TNEF framing.
///
/// ```
/// use std::io::{Cursor, Write};
/// use mimecraft::tnef::{RtfCompressedWriter, RtfCompressedToRtf, RtfCompressionMode};
///
/// let mut writer = RtfCompressedWriter::new(Cursor::new(Vec::new()),
///                                          RtfCompressionMode::Compressed)?;
/// writer.write_all(b"{\\rtf1 hello hello hello}")?;
/// writer.finish()?;
/// let bytes = writer.into_inner().into_inner();
/// let mut decoder = RtfCompressedToRtf::new();
/// let mut text = Vec::new();
/// decoder.filter(&bytes, &mut text);
/// decoder.validate()?;
/// assert_eq!(text, b"{\\rtf1 hello hello hello}");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
#[must_use = "call finish() to write the compressed RTF header and finalize output"]
pub struct RtfCompressedWriter<W> {
    inner: W,
    header_offset: u64,
    mode: RtfCompressionMode,
    state: State,
    raw_size: u64,
    content_size: u64,
    crc: u32,
    // The dictionary and its two-byte hash chains.
    dictionary: [u8; DICTIONARY_SIZE],
    head: [i16; DICTIONARY_SIZE],
    previous: [i16; DICTIONARY_SIZE],
    write_offset: usize,
    full: bool,
    // Input not yet encoded, and the current group of up to eight tokens.
    input: [u8; 4096],
    input_index: usize,
    input_end: usize,
    group: [u8; 1 + 8 * 2],
    group_length: usize,
    token_count: usize,
}

impl<W: Write + Seek> RtfCompressedWriter<W> {
    /// Reserves a header at the sink's current position and initializes a writer.
    ///
    /// # Errors
    ///
    /// Rejects `Unknown` modes before I/O. Propagates sink position/write errors;
    /// a failed header reservation may leave partial output.
    pub fn new(mut inner: W, mode: RtfCompressionMode) -> io::Result<Self> {
        if matches!(mode, RtfCompressionMode::Unknown(_)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported RTF compression mode",
            ));
        }
        let header_offset = retry(|| inner.stream_position())?;
        inner.write_all(&[0; HEADER_SIZE])?;
        let mut writer = Self {
            inner,
            header_offset,
            mode,
            state: State::Open,
            raw_size: 0,
            content_size: 0,
            crc: 0,
            dictionary: [0; DICTIONARY_SIZE],
            head: [-1; DICTIONARY_SIZE],
            previous: [-1; DICTIONARY_SIZE],
            write_offset: INITIALIZER.len(),
            full: false,
            input: [0; 4096],
            input_index: 0,
            input_end: 0,
            group: [0; 1 + 8 * 2],
            group_length: 1,
            token_count: 0,
        };
        if mode == RtfCompressionMode::Compressed {
            writer.dictionary[..INITIALIZER.len()].copy_from_slice(INITIALIZER);
            // Every initializer position followed by another initializer byte can begin a match.
            for position in 0..INITIALIZER.len() - 1 {
                writer.insert(position);
            }
        }
        Ok(writer)
    }

    /// Returns the destination, which may not yet contain buffered input.
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Returns the destination without flushing, finalizing, or discarding
    /// already written bytes. Any unencoded input/incomplete token group is lost.
    pub fn into_inner(self) -> W {
        self.inner
    }

    /// Compression mode selected at construction.
    pub fn compression_mode(&self) -> RtfCompressionMode {
        self.mode
    }

    /// Number of raw bytes accepted by successful writes.
    pub fn raw_size(&self) -> u64 {
        self.raw_size
    }

    /// Whether the header and terminator were successfully finalized and flushed.
    pub fn is_finished(&self) -> bool {
        self.state == State::Finished
    }

    /// Whether a failure during encoding or sink I/O prevents further operations.
    pub fn is_faulted(&self) -> bool {
        self.state == State::Faulted
    }

    /// Finishes input, patches the header, restores the end position, and flushes.
    ///
    /// Repeated successful calls are no-ops. Further nonempty writes fail.
    /// Unlike MimeKit's disposal callback, errors are returned directly rather
    /// than being caught during disposal.
    ///
    /// # Errors
    ///
    /// Propagates size, write, seek, and flush failures and faults the writer.
    /// Output remains provisional on error, and finalization cannot
    /// be retried. A faulted writer returns [`io::ErrorKind::BrokenPipe`].
    pub fn finish(&mut self) -> io::Result<()> {
        self.check_faulted()?;
        if self.state == State::Finished {
            return Ok(());
        }
        self.run(|writer| {
            if writer.mode == RtfCompressionMode::Compressed {
                writer.encode(true)?;
                // [MS-OXRTFCP] 2.1.3.1.4: A reference to the dictionary's current
                // write offset marks the end of the stream.
                writer.write_reference(writer.write_offset, MIN_MATCH)?;
                if writer.token_count > 0 {
                    writer.write_group()?;
                }
            }
            let header = make_header(
                writer.content_size,
                writer.raw_size,
                writer.mode,
                writer.crc,
            )?;
            let end = retry(|| writer.inner.stream_position())?;
            retry(|| writer.inner.seek(SeekFrom::Start(writer.header_offset)))?;
            writer.inner.write_all(&header)?;
            retry(|| writer.inner.seek(SeekFrom::Start(end)))?;
            retry(|| writer.inner.flush())
        })?;
        self.state = State::Finished;
        Ok(())
    }

    fn check_faulted(&self) -> io::Result<()> {
        if self.is_faulted() {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the compressed RTF writer is faulted",
            ))
        } else {
            Ok(())
        }
    }

    fn run<T>(&mut self, operation: impl FnOnce(&mut Self) -> io::Result<T>) -> io::Result<T> {
        let state = self.state;
        self.state = State::Faulted;
        let value = operation(self)?;
        self.state = state;
        Ok(value)
    }

    fn hash(a: u8, b: u8) -> usize {
        ((usize::from(a) << 4) ^ usize::from(b)) & DICTIONARY_MASK
    }

    // Adds a position to the hash chain of the two bytes that begin there.
    fn insert(&mut self, position: usize) {
        let hash = Self::hash(
            self.dictionary[position],
            self.dictionary[(position + 1) & DICTIONARY_MASK],
        );
        self.previous[position] = self.head[hash];
        self.head[hash] = position as i16;
    }

    // Adds a byte to the dictionary exactly as the decoder does.
    fn add_to_dictionary(&mut self, byte: u8) {
        self.dictionary[self.write_offset] = byte;
        // Now that its following byte is known, the previous position can begin a match.
        self.insert(self.write_offset.wrapping_sub(1) & DICTIONARY_MASK);
        self.write_offset = (self.write_offset + 1) & DICTIONARY_MASK;
        if self.write_offset == 0 {
            self.full = true;
        }
    }

    // The decoder adds copied bytes one at a time, so a match may overlap
    // the bytes it produces. Such positions come from the input, not old dictionary bytes.
    fn match_length(&self, offset: usize, maximum: usize) -> usize {
        let mut length = 0;
        while length < maximum {
            let position = (offset + length) & DICTIONARY_MASK;
            let distance = position.wrapping_sub(self.write_offset) & DICTIONARY_MASK;
            let byte = if distance < length {
                self.input[self.input_index + distance]
            } else {
                self.dictionary[position]
            };
            if byte != self.input[self.input_index + length] {
                break;
            }
            length += 1;
        }
        length
    }

    fn find_match(&self, available: usize) -> (usize, usize) {
        let maximum = available.min(MAX_MATCH);
        let mut candidate = self.head[Self::hash(
            self.input[self.input_index],
            self.input[self.input_index + 1],
        )];
        let (mut offset, mut length, mut age) = (0, 0, usize::MAX);
        // [MS-OXRTFCP] 2.3.3.2.1 scans from the oldest dictionary position and
        // keeps the first longest match: prefer the oldest candidate in a tie.
        let start = if self.full {
            (self.write_offset + 1) & DICTIONARY_MASK
        } else {
            0
        };
        for _ in 0..MAX_CHAIN {
            if candidate < 0 {
                break;
            }
            let position = candidate as usize;
            // A reference to the current write offset is reserved for the end marker.
            if position != self.write_offset {
                let candidate_length = self.match_length(position, maximum);
                let candidate_age = position.wrapping_sub(start) & DICTIONARY_MASK;
                if candidate_length > length
                    || (candidate_length == length && length > 0 && candidate_age < age)
                {
                    (offset, length, age) = (position, candidate_length, candidate_age);
                }
            }
            candidate = self.previous[position];
        }
        (offset, length)
    }

    fn write_group(&mut self) -> io::Result<()> {
        check_size(
            self.content_size,
            self.group_length as u64,
            MAX_CONTENT_SIZE,
        )?;
        self.inner.write_all(&self.group[..self.group_length])?;
        for &byte in &self.group[..self.group_length] {
            self.crc = crc32::update(self.crc, byte);
        }
        self.content_size += self.group_length as u64;
        self.group_length = 1;
        self.token_count = 0;
        self.group[0] = 0;
        Ok(())
    }

    fn write_token(&mut self, reference: bool, first: u8, second: u8) -> io::Result<()> {
        if reference {
            self.group[0] |= 1 << self.token_count;
        }
        self.group[self.group_length] = first;
        self.group_length += 1;
        if reference {
            self.group[self.group_length] = second;
            self.group_length += 1;
        }
        self.token_count += 1;
        if self.token_count == 8 {
            self.write_group()?;
        }
        Ok(())
    }

    fn write_reference(&mut self, offset: usize, length: usize) -> io::Result<()> {
        // A reference is a 12-bit offset followed by a 4-bit length minus two, big-endian.
        self.write_token(
            true,
            (offset >> 4) as u8,
            (((offset & 15) << 4) | (length - MIN_MATCH)) as u8,
        )
    }

    // Encode while more than MAX_MATCH bytes remain, unless the input is complete.
    fn encode(&mut self, final_input: bool) -> io::Result<()> {
        let minimum = if final_input { 0 } else { MAX_MATCH };
        while self.input_end - self.input_index > minimum {
            let available = self.input_end - self.input_index;
            let (offset, length) = if available >= MIN_MATCH {
                self.find_match(available)
            } else {
                (0, 0)
            };
            if length >= MIN_MATCH {
                self.write_reference(offset, length)?;
                for _ in 0..length {
                    self.add_to_dictionary(self.input[self.input_index]);
                    self.input_index += 1;
                }
            } else {
                let byte = self.input[self.input_index];
                self.write_token(false, byte, 0)?;
                self.input_index += 1;
                self.add_to_dictionary(byte);
            }
        }
        // Move the remaining lookahead to the start of the input buffer.
        self.input.copy_within(self.input_index..self.input_end, 0);
        self.input_end -= self.input_index;
        self.input_index = 0;
        Ok(())
    }
}

impl<W: Write + Seek> Write for RtfCompressedWriter<W> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.check_faulted()?;
        if input.is_empty() {
            return Ok(0);
        }
        if self.state == State::Finished {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "compressed RTF input is finished",
            ));
        }
        check_size(self.raw_size, input.len() as u64, MAX_RAW_SIZE)?;
        if self.mode == RtfCompressionMode::Uncompressed {
            check_size(self.content_size, input.len() as u64, MAX_CONTENT_SIZE)?;
            let count = self.run(|writer| {
                let count = retry(|| writer.inner.write(input))?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write RTF content",
                    ));
                }
                Ok(count)
            })?;
            self.raw_size += count as u64;
            self.content_size += count as u64;
            return Ok(count);
        }
        // Fallible encoding only processes input already acknowledged by a previous write.
        if self.input_end == self.input.len() {
            self.run(|writer| writer.encode(false))?;
        }
        let count = input.len().min(self.input.len() - self.input_end);
        self.input[self.input_end..self.input_end + count].copy_from_slice(&input[..count]);
        self.input_end += count;
        self.raw_size += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check_faulted()?;
        let encode = self.state == State::Open && self.mode == RtfCompressionMode::Compressed;
        self.run(|writer| {
            if encode {
                writer.encode(false)?;
            }
            retry(|| writer.inner.flush())
        })
    }
}

fn retry<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        match operation() {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}

fn check_size(current: u64, additional: u64, maximum: u64) -> io::Result<()> {
    if current > maximum || additional > maximum - current {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the compressed RTF is too large",
        ))
    } else {
        Ok(())
    }
}

fn make_header(
    content_size: u64,
    raw_size: u64,
    mode: RtfCompressionMode,
    crc: u32,
) -> io::Result<[u8; HEADER_SIZE]> {
    check_size(content_size, 0, MAX_CONTENT_SIZE)?;
    check_size(raw_size, 0, MAX_RAW_SIZE)?;
    let mut header = [0; HEADER_SIZE];
    // COMPSIZE includes the twelve bytes of the header that follow it.
    header[..4].copy_from_slice(&(content_size as u32 + 12).to_le_bytes());
    header[4..8].copy_from_slice(&(raw_size as u32).to_le_bytes());
    header[8..12].copy_from_slice(&u32::from(mode).to_le_bytes());
    let checksum = if mode == RtfCompressionMode::Compressed {
        crc
    } else {
        0
    };
    header[12..].copy_from_slice(&checksum.to_le_bytes());
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn wire_size_boundaries_are_checked_without_large_allocations() {
        for (maximum, other) in [
            (MAX_RAW_SIZE, MAX_CONTENT_SIZE),
            (MAX_CONTENT_SIZE, MAX_RAW_SIZE),
        ] {
            assert!(check_size(maximum - 1, 1, maximum).is_ok());
            assert!(check_size(maximum, 0, maximum).is_ok());
            for (current, additional) in [
                (maximum, 1),
                (maximum - 1, 2),
                (u64::MAX, 0),
                (other, u64::MAX),
            ] {
                assert_eq!(
                    check_size(current, additional, maximum).unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
            }
        }
        let header = make_header(
            MAX_CONTENT_SIZE,
            MAX_RAW_SIZE,
            RtfCompressionMode::Compressed,
            42,
        )
        .unwrap();
        assert_eq!(&header[..4], &i32::MAX.to_le_bytes());
        assert_eq!(&header[4..8], &i32::MAX.to_le_bytes());
        assert_eq!(&header[12..], &42u32.to_le_bytes());
        assert!(make_header(MAX_CONTENT_SIZE + 1, 0, RtfCompressionMode::Compressed, 0).is_err());
        assert!(make_header(0, MAX_RAW_SIZE + 1, RtfCompressionMode::Compressed, 0).is_err());
    }

    #[test]
    fn input_size_preflight_does_not_consume_input_or_fault() {
        for mode in [
            RtfCompressionMode::Compressed,
            RtfCompressionMode::Uncompressed,
        ] {
            let mut writer = RtfCompressedWriter::new(Cursor::new(Vec::new()), mode).unwrap();
            writer.raw_size = MAX_RAW_SIZE - 1;
            assert_eq!(
                writer.write(b"xx").unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(writer.raw_size(), MAX_RAW_SIZE - 1);
            assert_eq!(writer.get_ref().get_ref().len(), HEADER_SIZE);
            assert!(!writer.is_faulted());
            assert_eq!(writer.write(b"x").unwrap(), 1);
            assert_eq!(writer.raw_size(), MAX_RAW_SIZE);
        }
        let mut writer =
            RtfCompressedWriter::new(Cursor::new(Vec::new()), RtfCompressionMode::Uncompressed)
                .unwrap();
        writer.content_size = MAX_CONTENT_SIZE - 1;
        assert!(writer.write(b"xx").is_err());
        assert!(!writer.is_faulted());
        assert_eq!(writer.write(b"x").unwrap(), 1);
        assert_eq!(writer.content_size, MAX_CONTENT_SIZE);
    }

    #[test]
    fn encoded_content_overflow_faults_before_writing_a_group() {
        let mut writer =
            RtfCompressedWriter::new(Cursor::new(Vec::new()), RtfCompressionMode::Compressed)
                .unwrap();
        writer.content_size = MAX_CONTENT_SIZE - 2;
        // Even empty compressed input needs a flags byte and two-byte end marker.
        assert_eq!(
            writer.finish().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(writer.is_faulted());
        assert_eq!(writer.get_ref().get_ref(), &[0; HEADER_SIZE]);
    }
}

// Ported from MimeKit's Tnef/TnefWriter.cs and TnefValueWriteStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::HashMap,
    io::{self, Read, Seek, SeekFrom, Write},
};

use super::{
    SIGNATURE, TnefAttributeLevel as Level, TnefAttributeTag as Tag, TnefAttributeType as Type,
    TnefDateTime, TnefNameId, TnefPropertyWriter, VERSION, checksum,
};
use crate::charsets::{self, Charset};
use crate::io::MemoryBlockStream;

pub(super) fn get_charset(codepage: u32) -> io::Result<Charset> {
    // Codepage 0 (CP_ACP) would select a platform-dependent encoding.
    if matches!(codepage, 0 | 1200 | 1201 | 12000 | 12001) {
        return Err(invalid(
            "the TNEF codepage must not be zero or UTF-16/UTF-32",
        ));
    }
    charsets::from_codepage(codepage).ok_or_else(|| invalid("unsupported TNEF codepage"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    // Message-level attributes may be written.
    Message,
    // attMsgProps has been written, so only attachments may follow.
    MessageProperties,
    // attAttachRenderData has been written; attachment-level attributes may follow.
    Attachment,
    // attAttachment has been written, so only a new attachment may follow.
    AttachmentProperties,
    Finished,
    Faulted,
}

/// A synchronous, forward-only TNEF attribute writer.
///
/// Writes the signature, legacy key, version, and OEM codepage automatically on
/// the first attribute, [`flush`](Self::flush), or [`finish`](Self::finish).
/// Attribute levels, lengths, and checksums are calculated automatically.
/// Message attributes precede attachments; `MAPI_PROPERTIES` ends message
/// attributes, `ATTACH_RENDER_DATA` begins each attachment, and `ATTACHMENT`
/// ends its attributes. Unknown, null, and automatically written tags are rejected.
///
/// Raw values are not semantically validated, including MAPI property payloads.
/// Typed helpers check integer ranges, date types, and message-class strings.
/// Text uses the existing strict charset encoder, rejecting unmappable characters
/// rather than applying .NET's replacement/best-fit fallback.
///
/// No seekable output sink or whole-message buffer is needed.
/// [`write_attribute_from`](Self::write_attribute_from) copies a known-length
/// payload with a fixed 4 KiB buffer. [`open_attribute`](Self::open_attribute)
/// buffers unknown-length values in a memory block stream; custom memory/file
/// storage can be supplied with [`open_attribute_with_buffer`](Self::open_attribute_with_buffer).
/// [`open_properties`](Self::open_properties) constructs MAPI property lists
/// and recipient tables in the same buffered storage. Property writers can open
/// embedded messages with automatic Object framing and independent named IDs.
///
/// Call `finish` explicitly to observe final flush errors. Neither drop nor
/// [`into_inner`](Self::into_inner) performs writer I/O or finishes the stream;
/// an owned sink retains its normal drop behavior. Use `&mut sink` to retain it.
/// Validation errors leave state and output unchanged. Other I/O errors
/// permanently fault the writer, since partial framing cannot be retried.
///
/// ```
/// use mimecraft::tnef::{TnefAttributeTag as Tag, TnefReader, TnefWriter};
///
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0)?;
/// writer.write_string(Tag::SUBJECT, "Hello")?;
/// writer.write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])?;
/// writer.write_string(Tag::ATTACH_TITLE, "hello.txt")?;
/// writer.write_attribute(Tag::ATTACH_DATA, b"Hello world!")?;
/// writer.finish()?;
/// let bytes = writer.into_inner();
/// let mut reader = TnefReader::new(bytes.as_slice());
/// let mut count = 0;
/// while reader.read_next()? { count += 1; }
/// assert_eq!(count, 6); // Two automatic attributes and four supplied values.
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
#[must_use = "call finish() to emit the TNEF header and observe final flush errors"]
pub struct TnefWriter<W> {
    inner: W,
    charset: Charset,
    codepage: u32,
    legacy_key: u16,
    state: State,
    header_written: bool,
    named_ids: HashMap<TnefNameId, u16>,
}

impl<W: Write> TnefWriter<W> {
    /// Creates a writer without performing I/O. Use codepage 1252 for Western
    /// text and legacy key zero when no legacy key is needed.
    ///
    /// # Errors
    ///
    /// Rejects unsupported codepages, platform-dependent codepage zero, and
    /// UTF-16/UTF-32 codepages that encode characters using zero bytes.
    pub fn new(inner: W, codepage: u32, legacy_key: u16) -> io::Result<Self> {
        let charset = get_charset(codepage)?;
        Ok(Self {
            inner,
            charset,
            codepage,
            legacy_key,
            state: State::Message,
            header_written: false,
            named_ids: HashMap::new(),
        })
    }

    /// The explicitly selected codepage, also emitted in `OEM_CODEPAGE`.
    pub fn codepage(&self) -> u32 {
        self.codepage
    }

    /// The legacy key emitted immediately after the signature.
    pub fn legacy_key(&self) -> u16 {
        self.legacy_key
    }

    /// Observes the sink without allowing writes that bypass TNEF framing.
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Returns the sink without writing the header, flushing, or finishing.
    pub fn into_inner(self) -> W {
        self.inner
    }

    /// Whether finalization and its flush completed successfully.
    pub fn is_finished(&self) -> bool {
        self.state == State::Finished
    }

    /// Whether an I/O failure or unfinished buffered attribute prevents further operations.
    pub fn is_faulted(&self) -> bool {
        self.state == State::Faulted
    }

    /// Opens a MAPI property list or recipient table in a memory block stream.
    ///
    /// # Errors
    ///
    /// Only `MAPI_PROPERTIES`, `ATTACHMENT`, and `RECIPIENT_TABLE` are accepted.
    /// Attribute ordering and completion follow [`open_attribute`](Self::open_attribute).
    pub fn open_properties(&mut self, tag: Tag) -> io::Result<TnefPropertyWriter<'_, W>> {
        self.open_properties_with_buffer(tag, MemoryBlockStream::new())
    }

    /// Opens a property writer with owned or borrowed, empty scratch storage.
    ///
    /// # Errors
    ///
    /// Shares [`open_properties`](Self::open_properties) and
    /// [`open_attribute_with_buffer`](Self::open_attribute_with_buffer) validation.
    /// After opening, scratch write errors fault the parent, including failure
    /// to write the initial property/row-count placeholder.
    pub fn open_properties_with_buffer<B: Read + Write + Seek>(
        &mut self,
        tag: Tag,
        buffer: B,
    ) -> io::Result<TnefPropertyWriter<'_, W, B>> {
        self.next_attribute(tag)?;
        if !matches!(
            tag,
            Tag::MAPI_PROPERTIES | Tag::ATTACHMENT | Tag::RECIPIENT_TABLE
        ) {
            return Err(invalid("the attribute does not contain MAPI properties"));
        }
        let charset = self.charset;
        let stream = self.open_attribute_with_buffer(tag, buffer)?;
        TnefPropertyWriter::new(stream, charset, tag == Tag::RECIPIENT_TABLE)
    }

    /// Opens an unknown-length raw attribute backed by a [`MemoryBlockStream`].
    ///
    /// The child exclusively borrows this writer. Call its `finish` to emit the
    /// attribute and release the borrow; dropping it unfinished faults this writer.
    /// No output is written until the child finishes.
    ///
    /// # Errors
    ///
    /// Rejects the same tags/states as [`write_attribute`](Self::write_attribute).
    pub fn open_attribute(&mut self, tag: Tag) -> io::Result<TnefAttributeWriter<'_, W>> {
        self.open_attribute_with_buffer(tag, MemoryBlockStream::new())
    }

    /// Opens an unknown-length raw attribute with caller-supplied scratch storage.
    ///
    /// The buffer must be empty and positioned at zero, and support `Read`,
    /// `Write`, and `Seek`. It is owned by the child and returned on successful
    /// finalization. Pass `&mut buffer` to retain it even on error. File creation
    /// and deletion policies belong to the caller; no factory or implicit spill
    /// threshold is used.
    ///
    /// # Errors
    ///
    /// Rejects invalid tags/states before accessing the buffer. Buffer validation
    /// errors leave the parent state and output unchanged, though failed seeks
    /// can change the scratch position. Interrupted operations are retried.
    pub fn open_attribute_with_buffer<B: Read + Write + Seek>(
        &mut self,
        tag: Tag,
        mut buffer: B,
    ) -> io::Result<TnefAttributeWriter<'_, W, B>> {
        self.next_attribute(tag)?;
        // Each temporary stream must be new, empty, and positioned at zero.
        if retry(|| buffer.stream_position())? != 0 {
            return Err(invalid(
                "the TNEF attribute buffer must be positioned at zero",
            ));
        }
        let length = retry(|| buffer.seek(SeekFrom::End(0)))?;
        retry(|| buffer.rewind())?;
        if length != 0 {
            return Err(invalid("the TNEF attribute buffer must be empty"));
        }
        let previous_state = self.state;
        // Poison by default: abandoning or forgetting the child must not let the
        // parent report success with a silently omitted attribute. No Drop I/O.
        self.state = State::Faulted;
        Ok(TnefAttributeWriter {
            parent: self,
            buffer,
            tag,
            previous_state,
            length: 0,
            faulted: false,
        })
    }

    /// Writes a raw attribute value without validating its contents.
    ///
    /// # Errors
    ///
    /// Rejects invalid tags/order, values longer than `i32::MAX`, and finished
    /// writers before I/O. Sink errors are propagated and fault the writer.
    pub fn write_attribute(&mut self, tag: Tag, value: &[u8]) -> io::Result<()> {
        self.write_parts(tag, value, false)
    }

    /// Writes exactly `length` raw bytes from a forward-only source.
    ///
    /// Does not read beyond the declared length or test for EOF afterward.
    /// Zero-length values never read the source. Both sources and sinks may
    /// return short reads/writes; interrupted operations are retried.
    ///
    /// # Errors
    ///
    /// Invalid tags/order and lengths exceeding `i32::MAX` leave both streams
    /// untouched. Premature EOF returns [`io::ErrorKind::UnexpectedEof`].
    /// Read/write failures fault the writer and leave provisional output.
    pub fn write_attribute_from<R: Read + ?Sized>(
        &mut self,
        tag: Tag,
        source: &mut R,
        length: u64,
    ) -> io::Result<()> {
        let (level, state) = self.next_attribute(tag)?;
        let length = attribute_length(length)?;
        self.run(|writer| {
            writer.write_header()?;
            writer
                .inner
                .write_all(&attribute_header(level, tag, length))?;
            let mut buffer = [0; 4096];
            let mut remaining = length;
            let mut sum = 0;
            while remaining != 0 {
                let count = (remaining as usize).min(buffer.len());
                let read = loop {
                    match source.read(&mut buffer[..count]) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        result => break result?,
                    }
                };
                if read == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "incomplete TNEF attribute source",
                    ));
                }
                writer.inner.write_all(&buffer[..read])?;
                sum = checksum(sum, &buffer[..read]);
                remaining -= read as u32;
            }
            writer.inner.write_all(&sum.to_le_bytes())
        })?;
        self.state = state;
        Ok(())
    }

    /// Writes a signed 16-bit or 32-bit attribute, or an unsigned 16-bit Word.
    /// DWord values use the supplied `i32` bit pattern, as in MimeKit.
    ///
    /// # Errors
    ///
    /// Rejects noninteger tags and out-of-range 16-bit values without I/O.
    /// Otherwise returns the same errors as [`write_attribute`](Self::write_attribute).
    pub fn write_i32(&mut self, tag: Tag, value: i32) -> io::Result<()> {
        self.next_attribute(tag)?;
        let bytes = value.to_le_bytes();
        let length = match tag.attribute_type() {
            Type::Short if i16::try_from(value).is_ok() => 2,
            Type::Word if !is_message_class(tag) && u16::try_from(value).is_ok() => 2,
            Type::Long | Type::DWord => 4,
            _ => return Err(invalid("invalid integer attribute type or range")),
        };
        self.write_attribute(tag, &bytes[..length])
    }

    /// Writes a NUL-terminated 8-bit string in the configured charset.
    /// Message classes require 1-254 printable ASCII characters.
    ///
    /// # Errors
    ///
    /// Rejects nonstring tags, invalid message classes, and unmappable text
    /// without I/O. Otherwise returns the errors of [`write_attribute`](Self::write_attribute).
    pub fn write_string(&mut self, tag: Tag, value: &str) -> io::Result<()> {
        self.next_attribute(tag)?;
        if is_message_class(tag) {
            // [MS-OXOMSG] restricts the message class to printable ASCII characters.
            if value.is_empty()
                || value.len() > 254
                || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
            {
                return Err(invalid(
                    "message classes require 1-254 printable ASCII characters",
                ));
            }
        } else if !matches!(tag.attribute_type(), Type::String | Type::Text) {
            return Err(invalid("the TNEF attribute is not a string"));
        }
        let bytes = self
            .charset
            .encode(value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        self.write_parts(tag, &bytes, true)
    }

    /// Writes calendar components without converting the time kind or zone.
    /// Fractional seconds are omitted, as in MimeKit's legacy date writer.
    ///
    /// # Errors
    ///
    /// Rejects nondate tags without I/O. Otherwise returns the errors of
    /// [`write_attribute`](Self::write_attribute).
    pub fn write_datetime(&mut self, tag: Tag, value: TnefDateTime) -> io::Result<()> {
        self.next_attribute(tag)?;
        if tag.attribute_type() != Type::Date {
            return Err(invalid("the TNEF attribute is not a date"));
        }
        // The TNEF date structure is seven 16-bit values: year, month, day,
        // hour, minute, second, and day-of-week (Sunday is zero).
        let weekday = ((value.ticks() / 864_000_000_000 + 1) % 7) as u16;
        let components = [
            value.year(),
            u16::from(value.month()),
            u16::from(value.day()),
            u16::from(value.hour()),
            u16::from(value.minute()),
            u16::from(value.second()),
            weekday,
        ];
        let mut bytes = [0; 14];
        for (output, component) in bytes.as_chunks_mut::<2>().0.iter_mut().zip(components) {
            output.copy_from_slice(&component.to_le_bytes());
        }
        self.write_attribute(tag, &bytes)
    }

    /// Emits the automatic header if needed and flushes the sink without
    /// ending input. May also flush an already successfully finished writer.
    ///
    /// # Errors
    ///
    /// Sink errors fault the writer. Faulted writers return `BrokenPipe`.
    pub fn flush(&mut self) -> io::Result<()> {
        self.check_faulted()?;
        self.run(|writer| {
            writer.write_header()?;
            loop {
                match writer.inner.flush() {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    result => return result,
                }
            }
        })
    }

    /// Emits the header, flushes, and ends input. Successful repeats are no-ops.
    /// TNEF has no additional end-of-stream marker.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`flush`](Self::flush); failures cannot be retried.
    pub fn finish(&mut self) -> io::Result<()> {
        self.check_faulted()?;
        if !self.is_finished() {
            self.flush()?;
            self.state = State::Finished;
        }
        Ok(())
    }

    fn check_faulted(&self) -> io::Result<()> {
        if self.is_faulted() {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the TNEF writer is faulted",
            ))
        } else {
            Ok(())
        }
    }

    // Validate the grammar before changing state or emitting any bytes.
    fn next_attribute(&self, tag: Tag) -> io::Result<(Level, State)> {
        self.check_faulted()?;
        if self.is_finished() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "TNEF input is finished",
            ));
        }
        if !tag.is_known() || matches!(tag, Tag::NULL | Tag::TNEF_VERSION | Tag::OEM_CODEPAGE) {
            return Err(invalid(
                "the TNEF attribute is unknown, null, or written automatically",
            ));
        }
        let level = if tag.is_attachment() {
            Level::Attachment
        } else {
            Level::Message
        };
        let state = match tag {
            Tag::MAPI_PROPERTIES if self.state == State::Message => State::MessageProperties,
            // attAttachRenderData begins a new attachment.
            Tag::ATTACH_RENDER_DATA => State::Attachment,
            Tag::ATTACHMENT if self.state == State::Attachment => State::AttachmentProperties,
            Tag::MAPI_PROPERTIES | Tag::ATTACHMENT => {
                return Err(invalid(
                    "property attributes must end their message or attachment",
                ));
            }
            _ if (level == Level::Message && self.state == State::Message)
                || (level == Level::Attachment && self.state == State::Attachment) =>
            {
                self.state
            }
            _ => return Err(invalid("invalid TNEF attribute order")),
        };
        Ok((level, state))
    }

    fn run(&mut self, operation: impl FnOnce(&mut Self) -> io::Result<()>) -> io::Result<()> {
        let state = self.state;
        self.state = State::Faulted;
        operation(self)?;
        self.state = state;
        Ok(())
    }

    fn write_parts(&mut self, tag: Tag, bytes: &[u8], terminated: bool) -> io::Result<()> {
        let (level, state) = self.next_attribute(tag)?;
        let length = attribute_length(bytes.len() as u64 + u64::from(terminated))?;
        self.run(|writer| {
            writer.write_header()?;
            writer
                .inner
                .write_all(&attribute_header(level, tag, length))?;
            writer.inner.write_all(bytes)?;
            if terminated {
                writer.inner.write_all(&[0])?;
            }
            writer.inner.write_all(&checksum(0, bytes).to_le_bytes())
        })?;
        self.state = state;
        Ok(())
    }

    fn write_header(&mut self) -> io::Result<()> {
        if self.header_written {
            return Ok(());
        }
        // [MS-OXTNEF] 2.1.3.1:
        // TNEFStream = TNEFHeader TNEFVersion OEMCodePage MessageData *AttachData
        let mut bytes = [0; 40];
        bytes[..4].copy_from_slice(&SIGNATURE.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.legacy_key.to_le_bytes());
        bytes[6..15].copy_from_slice(&attribute_header(Level::Message, Tag::TNEF_VERSION, 4));
        bytes[15..19].copy_from_slice(&VERSION.to_le_bytes());
        let sum = checksum(0, &bytes[15..19]);
        bytes[19..21].copy_from_slice(&sum.to_le_bytes());
        // The secondary codepage is unused and SHOULD be zero.
        bytes[21..30].copy_from_slice(&attribute_header(Level::Message, Tag::OEM_CODEPAGE, 8));
        bytes[30..34].copy_from_slice(&self.codepage.to_le_bytes());
        let sum = checksum(0, &bytes[30..38]);
        bytes[38..40].copy_from_slice(&sum.to_le_bytes());
        self.inner.write_all(&bytes)?;
        self.header_written = true;
        Ok(())
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn is_message_class(tag: Tag) -> bool {
    matches!(tag, Tag::MESSAGE_CLASS | Tag::ORIGINAL_MESSAGE_CLASS)
}

fn attribute_length(length: u64) -> io::Result<u32> {
    if length > i32::MAX as u64 {
        Err(invalid(
            "TNEF attribute values cannot exceed i32::MAX bytes",
        ))
    } else {
        Ok(length as u32)
    }
}

fn attribute_header(level: Level, tag: Tag, length: u32) -> [u8; 9] {
    let mut bytes = [0; 9];
    bytes[0] = match level {
        Level::Message => 1,
        Level::Attachment => 2,
        Level::Unknown(value) => value,
    };
    bytes[1..5].copy_from_slice(&tag.0.to_le_bytes());
    bytes[5..].copy_from_slice(&length.to_le_bytes());
    bytes
}

/// An append-only buffered TNEF attribute value, opened by [`TnefWriter`].
///
/// Implements `Write`, but not `Read` or `Seek`. Its position is always its
/// accepted byte count, [`len`](Self::len). `flush` flushes scratch storage only;
/// it does not emit an attribute or flush the parent output.
///
/// [`finish`](Self::finish) consumes the child, flushes/rewinds its buffer, and
/// emits the header, value, and checksum to the parent. Unlike MimeKit's disposal
/// callback/queue, output errors are returned at this call. The parent still
/// needs its own `finish` to flush the output sink.
///
/// Dropping, forgetting, or dismantling an unfinished child performs no writer
/// I/O and leaves the parent faulted, rather than silently discarding a value.
/// Owned buffers retain their normal drop behavior. Buffer/output errors are
/// terminal; oversized input is rejected before I/O and leaves the child usable.
///
/// ```
/// use std::io::Write;
/// use mimecraft::tnef::{TnefAttributeTag as Tag, TnefWriter};
///
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0)?;
/// let mut value = writer.open_attribute(Tag::BODY)?;
/// value.write_all(b"hello")?;
/// value.write_all(b" world\0")?;
/// let buffer = value.finish()?;
/// assert_eq!(buffer.len(), 12);
/// writer.finish()?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// The parent cannot advance while a child is in use:
///
/// ```compile_fail
/// use std::io::Write;
/// use mimecraft::tnef::{TnefAttributeTag as Tag, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
/// let mut value = writer.open_attribute(Tag::BODY).unwrap();
/// writer.flush().unwrap();
/// value.write_all(b"body").unwrap();
/// ```
#[derive(Debug)]
#[must_use = "call finish() or the parent TNEF writer remains faulted"]
pub struct TnefAttributeWriter<'a, W, B = MemoryBlockStream> {
    parent: &'a mut TnefWriter<W>,
    buffer: B,
    tag: Tag,
    previous_state: State,
    length: u64,
    faulted: bool,
}

impl<W: Write, B: Read + Write + Seek> TnefAttributeWriter<'_, W, B> {
    pub(super) fn codepage(&self) -> u32 {
        self.parent.codepage
    }

    pub(super) fn named_id(&mut self, name: &TnefNameId) -> io::Result<u16> {
        if let Some(&id) = self.parent.named_ids.get(name) {
            return Ok(id);
        }
        if self.parent.named_ids.len() >= 0x7fff {
            return Err(invalid(
                "the TNEF stream contains too many named properties",
            ));
        }
        let id = 0x8000 + self.parent.named_ids.len() as u16;
        self.parent.named_ids.insert(name.clone(), id);
        Ok(id)
    }

    pub(super) fn patch_u32(&mut self, offset: u64, value: u32) -> io::Result<()> {
        self.patch_bytes(offset, &value.to_le_bytes())
    }

    pub(super) fn patch_bytes(&mut self, offset: u64, value: &[u8]) -> io::Result<()> {
        self.check_faulted()?;
        if offset > self.length || value.len() as u64 > self.length - offset {
            return Err(invalid("the TNEF patch is outside the buffered attribute"));
        }
        let end = self.length;
        self.run(|buffer| {
            retry(|| buffer.seek(SeekFrom::Start(offset)))?;
            buffer.write_all(value)?;
            retry(|| buffer.seek(SeekFrom::Start(end)))?;
            Ok(())
        })
    }

    /// Number of bytes acknowledged by successful writes.
    pub fn len(&self) -> u64 {
        self.length
    }

    /// Whether no bytes have been accepted.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Whether buffer I/O has failed and prevents further operations.
    pub fn is_faulted(&self) -> bool {
        self.faulted
    }

    /// Observes the scratch buffer without allowing writes that bypass accounting.
    pub fn get_ref(&self) -> &B {
        &self.buffer
    }

    /// Abandons this attribute and returns its buffer without I/O.
    /// The parent remains faulted and cannot finish successfully.
    pub fn into_inner(self) -> B {
        self.buffer
    }

    /// Emits the complete attribute, releases the parent, and returns the buffer
    /// positioned at the end of the value. Does not flush the parent output.
    ///
    /// # Errors
    ///
    /// Propagates buffer flush/seek/read and parent write errors. A changed
    /// backing length is `InvalidData`. Errors leave the parent faulted and
    /// drop an owned buffer; borrow the buffer to retain it on failure.
    pub fn finish(mut self) -> io::Result<B> {
        self.flush()?;
        // Completing a buffered attribute must not silently omit or include
        // bytes if its backing stream no longer matches the acknowledged length.
        let length = retry(|| self.buffer.seek(SeekFrom::End(0)))?;
        if length != self.length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the TNEF attribute buffer length changed",
            ));
        }
        retry(|| self.buffer.rewind())?;
        self.parent.state = self.previous_state;
        self.parent
            .write_attribute_from(self.tag, &mut self.buffer, length)?;
        Ok(self.buffer)
    }

    pub(super) fn check_faulted(&self) -> io::Result<()> {
        if self.faulted {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the TNEF attribute writer is faulted",
            ))
        } else {
            Ok(())
        }
    }

    fn run<T>(&mut self, operation: impl FnOnce(&mut B) -> io::Result<T>) -> io::Result<T> {
        self.faulted = true;
        let value = operation(&mut self.buffer)?;
        self.faulted = false;
        Ok(value)
    }
}

impl<W: Write, B: Read + Write + Seek> Write for TnefAttributeWriter<'_, W, B> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.check_faulted()?;
        if input.is_empty() {
            return Ok(0);
        }
        if input.len() as u64 > i32::MAX as u64 - self.length {
            return Err(invalid(
                "TNEF attribute values cannot exceed i32::MAX bytes",
            ));
        }
        let count = self.run(|buffer| {
            let count = retry(|| buffer.write(input))?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to buffer TNEF data",
                ));
            }
            Ok(count)
        })?;
        self.length += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check_faulted()?;
        self.run(|buffer| retry(|| buffer.flush()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_ids_cover_the_wire_range_and_existing_names_survive_exhaustion() {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut stream = writer.open_attribute(Tag::BODY).unwrap();
        let name = |id| TnefNameId {
            guid: [0; 16],
            name: super::super::TnefName::Id(id),
        };
        for id in 0..0x7fff {
            assert_eq!(stream.named_id(&name(id)).unwrap(), 0x8000 + id as u16);
        }
        assert_eq!(
            stream.named_id(&name(0x7fff)).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(stream.named_id(&name(0)).unwrap(), 0x8000);
        assert_eq!(stream.named_id(&name(0x7ffe)).unwrap(), 0xfffe);
        assert!(!stream.is_faulted());
        stream.finish().unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn attribute_length_boundaries_do_not_require_large_allocations() {
        for length in [0, 1, i32::MAX as u64] {
            assert_eq!(attribute_length(length).unwrap(), length as u32);
        }
        for length in [i32::MAX as u64 + 1, u32::MAX as u64, u64::MAX] {
            assert_eq!(
                attribute_length(length).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn buffered_attribute_size_preflight_preserves_state() {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut value = writer.open_attribute(Tag::BODY).unwrap();
        value.length = i32::MAX as u64 - 1;
        assert_eq!(
            value.write(&[1, 2]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!value.is_faulted());
        assert!(value.get_ref().is_empty());
        assert_eq!(value.write(&[1]).unwrap(), 1);
        assert_eq!(value.len(), i32::MAX as u64);
        assert_eq!(
            value.write(&[2]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(value.write(&[]).unwrap(), 0);
        assert!(!value.is_faulted());
        value.length = 1;
        value.finish().unwrap();
        writer.finish().unwrap();
        assert!(!writer.is_faulted());
    }
}

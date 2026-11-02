// Ported from MimeKit's Tnef/TnefPropertyWriter.cs and TnefValueWriteStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    io::{self, Read, Seek, Write},
};

use super::{
    RtfCompressionMode, TnefAttributeWriter, TnefEmbeddedSink, TnefEmbeddedWriter, TnefName,
    TnefNameId, TnefProperty, TnefPropertyTag as Tag, TnefPropertyType as Type,
    TnefRtfPropertyWriter, TnefValue as Value, embedded_writer::EmbeddedOutput,
    property_types::Width,
};
use crate::{charsets::Charset, io::MemoryBlockStream};

/// A buffered, forward-only MAPI property or recipient-row writer.
///
/// Obtain this child from [`super::TnefWriter::open_properties`]. Begin each
/// property with `write_tag` or `write_named_tag`, then write its native values.
/// A scalar needs exactly one value; Null has none, and multivalues may be empty.
/// Recipient tables require `begin_row` before writing any properties.
/// Counts, variable lengths, terminators, and DWORD padding are automatic.
///
/// `write_value` uses [`Value`]'s exact native types, without numeric coercion:
/// currency is a scaled integer, dates are checked raw FILETIME/OLE numbers,
/// and object bytes include their 16-byte interface IID. Synthesized `DateTime`
/// values are not wire-native and are rejected. String8 uses the parent's
/// codepage and strict encoding, not .NET replacement/best-fit fallback.
///
/// `finish` consumes this child, emits the attribute immediately, and returns
/// scratch storage. Call the parent's `finish` separately. Dropping, forgetting,
/// or recovering storage from an unfinished child leaves the parent faulted.
/// No writer I/O occurs on drop. Validation errors leave the child usable;
/// scratch/output I/O failures are terminal. Borrow storage to retain it on error.
/// [`open_value`](Self::open_value) streams binary/object bytes into the same
/// scratch storage. [`open_rtf_compressed`](Self::open_rtf_compressed) encodes RTF
/// into a Binary value. [`open_embedded_message`](Self::open_embedded_message)
/// writes a nested TNEF message into an Object value with automatic IMessage IID.
///
/// ```
/// use mimecraft::tnef::{TnefAttributeTag as A, TnefPropertyTag as P, TnefValue, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0)?;
/// let mut properties = writer.open_properties(A::MAPI_PROPERTIES)?;
/// properties.write_tag(P::SUBJECT_W)?;
/// properties.write_value(&TnefValue::Text("Hello".into()))?;
/// properties.finish()?;
/// writer.finish()?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// The parent cannot advance while the property writer is in use:
///
/// ```compile_fail
/// use mimecraft::tnef::{TnefAttributeTag, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
/// let properties = writer.open_properties(TnefAttributeTag::MAPI_PROPERTIES).unwrap();
/// writer.finish().unwrap();
/// properties.finish().unwrap();
/// ```
#[derive(Debug)]
#[must_use = "call finish() or the parent TNEF writer remains faulted"]
pub struct TnefPropertyWriter<'a, W, B = MemoryBlockStream> {
    stream: TnefAttributeWriter<'a, W, B>,
    charset: Charset,
    table: bool,
    // Offset of the current property count, or None before the first row.
    count_offset: Option<u64>,
    property_count: u32,
    row_count: u32,
    current: Option<Current>,
    value_open: bool,
}

#[derive(Debug)]
struct Current {
    tag: Tag,
    values: u32,
    count_offset: Option<u64>,
}

impl<'a, W: Write, B: Read + Write + Seek> TnefPropertyWriter<'a, W, B> {
    pub(super) fn new(
        mut stream: TnefAttributeWriter<'a, W, B>,
        charset: Charset,
        table: bool,
    ) -> io::Result<Self> {
        // Both a property list and a table begin with a 32-bit count
        // (of properties or rows).
        stream.write_all(&[0; 4])?;
        Ok(Self {
            stream,
            charset,
            table,
            count_offset: if table { None } else { Some(0) },
            property_count: 0,
            row_count: 0,
            current: None,
            value_open: false,
        })
    }

    /// Whether this attribute contains recipient rows.
    pub fn is_table(&self) -> bool {
        self.table
    }
    /// Number of properties started in this list or the current row.
    pub fn property_count(&self) -> u32 {
        self.property_count
    }
    /// Number of recipient rows started, including empty rows.
    pub fn row_count(&self) -> u32 {
        self.row_count
    }
    /// Whether failed scratch I/O or an unfinished value prevents further operations.
    pub fn is_faulted(&self) -> bool {
        self.value_open || self.stream.is_faulted()
    }
    /// Immutably borrows scratch storage; do not modify it through shared handles.
    pub fn get_ref(&self) -> &B {
        self.stream.get_ref()
    }
    /// Abandons the attribute without I/O; the parent remains faulted.
    pub fn into_inner(self) -> B {
        self.stream.into_inner()
    }

    /// Completes the previous row and starts a recipient row.
    ///
    /// # Errors
    ///
    /// Rejects non-table attributes, incomplete scalar properties, and count or
    /// length overflow before I/O. Scratch errors are terminal.
    pub fn begin_row(&mut self) -> io::Result<()> {
        self.check()?;
        if !self.table {
            return Err(invalid("only a recipient table contains rows"));
        }
        self.check_complete()?;
        let count = increment(self.row_count)?;
        self.reserve(4)?;
        self.end_row()?;
        let offset = self.stream.len();
        self.stream.write_all(&[0; 4])?;
        self.count_offset = Some(offset);
        self.row_count = count;
        self.property_count = 0;
        Ok(())
    }

    /// Completes the previous property and begins an ordinary property.
    ///
    /// # Errors
    ///
    /// Rejects named tags (use `write_named_tag`), unsupported types, forbidden
    /// multivalue types, missing rows, incomplete scalars, and size/count limits.
    pub fn write_tag(&mut self, tag: Tag) -> io::Result<()> {
        self.check()?;
        if tag.is_named() {
            return Err(invalid("named properties require a name descriptor"));
        }
        self.check_tag(tag, 0)?;
        self.start(tag, &[])
    }

    /// Begins a named property, allocating or reusing a stream-local ID.
    ///
    /// Names are case-sensitive; GUID/name pairs share IDs across property
    /// attributes and rows, independent of the value type.
    ///
    /// # Errors
    ///
    /// Shares `write_tag` validation, and rejects unknown name kinds or exhausted
    /// IDs (0x8000 through 0xfffe). No ID is assigned for validation failures.
    pub fn write_named_tag(&mut self, name: &TnefNameId, kind: Type) -> io::Result<()> {
        self.check()?;
        let descriptor = name_bytes(name)?;
        let tag = Tag::new(0x8000, kind);
        self.check_tag(tag, descriptor.len() as u64)?;
        let id = self.stream.named_id(name)?;
        self.start(Tag::new(id, kind), &descriptor)
    }

    /// Appends an exact native value to the current property.
    ///
    /// # Errors
    ///
    /// Rejects missing tags, mismatched native variants, invalid dates, unmappable
    /// String8, short objects, a second scalar value, and size/count limits before
    /// writing. Null properties have no value to append.
    pub fn write_value(&mut self, value: &Value) -> io::Result<()> {
        let current = self.check_value()?;
        let count = increment(current.values)?;
        let encoded = encode(self.charset, current.tag.value_type(), value)?;
        self.reserve(encoded.wire_length())?;
        self.emit(&encoded)?;
        self.current.as_mut().unwrap().values = count;
        Ok(())
    }

    /// Opens an append-only stream for one Binary or Object value.
    ///
    /// Bytes go directly into this property's attribute buffer, without a
    /// second scratch buffer. Object bytes must include a 16-byte interface IID.
    /// Explicitly finish the value before continuing properties or rows.
    ///
    /// # Errors
    ///
    /// Rejects missing/wrong tags, a second scalar value, count/size limits, and
    /// faulted writers before I/O. Failure to write the length placeholder is
    /// terminal. Dropping or forgetting an unfinished value faults this writer.
    pub fn open_value(&mut self) -> io::Result<TnefPropertyValueWriter<'_, 'a, W, B>> {
        let current = self.check_value()?;
        let minimum = match current.tag.value_type() {
            Type::BINARY => 0,
            Type::OBJECT => 16,
            _ => return Err(invalid("only Binary and Object values can be streamed")),
        };
        increment(current.values)?;
        self.reserve(4 + minimum)?;
        let offset = self.stream.len();
        self.stream.write_all(&[0; 4])?;
        // Keep the parent blocked even if the child is forgotten without Drop.
        self.value_open = true;
        Ok(TnefPropertyValueWriter {
            parent: self,
            offset,
        })
    }

    /// Opens an encoder for one Binary value containing LZFu or MELA RTF.
    ///
    /// Use [`RtfCompressionMode::Compressed`] to compress or `Uncompressed` to
    /// store raw RTF inside an MS-OXRTFCP header. The property need not be
    /// `RTF_COMPRESSED`; Binary multivalues are also supported.
    /// Explicitly finish the encoder, this property writer, and its parent.
    ///
    /// # Errors
    ///
    /// Rejects non-Binary properties, unknown modes, scalar/count violations,
    /// and insufficient minimum capacity before opening a value. Header write
    /// failures leave this property writer blocked.
    pub fn open_rtf_compressed(
        &mut self,
        mode: RtfCompressionMode,
    ) -> io::Result<TnefRtfPropertyWriter<'_, 'a, W, B>> {
        if self.check_value()?.tag.value_type() != Type::BINARY {
            return Err(invalid("only Binary properties can contain compressed RTF"));
        }
        let minimum = match mode {
            // Empty LZFu adds a flag byte and end-reference to its 16-byte
            // header, then pads the completed property value to a DWORD.
            RtfCompressionMode::Compressed => 20,
            RtfCompressionMode::Uncompressed => 16,
            RtfCompressionMode::Unknown(_) => {
                return Err(invalid("unsupported RTF compression mode"));
            }
        };
        self.reserve(4 + minimum)?;
        TnefRtfPropertyWriter::new(self.open_value()?, mode)
    }

    /// Opens an embedded TNEF message in an Object value, inheriting the codepage.
    ///
    /// Writes the IMessage IID automatically. Call the child's consuming
    /// `finish` before continuing this property's attribute.
    ///
    /// # Errors
    ///
    /// Shares [`open_embedded_message_with_codepage`](Self::open_embedded_message_with_codepage)
    /// errors, except that the inherited codepage is already validated.
    pub fn open_embedded_message(&mut self) -> io::Result<TnefEmbeddedWriter<'_>> {
        self.open_embedded_message_with_codepage(self.stream.codepage())
    }

    /// Opens an embedded TNEF message with an explicit codepage and legacy key zero.
    ///
    /// Uses the enclosing attribute scratch as its forward-only sink. Nested
    /// attributes may use their own default memory blocks or caller-supplied
    /// scratch. Each message has its own named-property ID table, as in MimeKit.
    ///
    /// # Errors
    ///
    /// Rejects non-Object properties, invalid codepages, scalar/count violations,
    /// and insufficient capacity for an empty message before mutation. Scratch
    /// failures and abandoned children leave the property writer blocked.
    pub fn open_embedded_message_with_codepage(
        &mut self,
        codepage: u32,
    ) -> io::Result<TnefEmbeddedWriter<'_>> {
        if self.check_value()?.tag.value_type() != Type::OBJECT {
            return Err(invalid(
                "only Object properties can contain embedded messages",
            ));
        }
        // Validate the codepage before beginning the value. The embedded writer
        // itself writes nothing until used; an invalid open must leave us usable.
        super::writer::get_charset(codepage)?;
        // Value length, IMessage IID, and the minimum TNEF header/attributes.
        self.reserve(4 + 16 + 40)?;
        let mut value = self.open_value()?;
        value.write_all(&super::MESSAGE_IID)?;
        TnefEmbeddedWriter::new(TnefEmbeddedSink::new(value.parent, value.offset), codepage)
    }

    /// Copies a complete owned snapshot, remapping named IDs and re-encoding text.
    ///
    /// # Errors
    ///
    /// Validates all values, names, counts, and total size before changing the
    /// buffer or allocating an ID. Invalid native values, names, or cardinality
    /// are rejected; data already lost by the reader cannot be reconstructed.
    /// Reader Null snapshots (`[Value::Null]`) and empty Null snapshots both
    /// produce a tag with no wire value.
    pub fn write_property(&mut self, property: &TnefProperty) -> io::Result<()> {
        self.check()?;
        let tag = property.tag();
        validate_type(tag)?;
        let descriptor = match property.name() {
            Some(name) => name_bytes(name)?,
            None if tag.is_named() => return Err(invalid("the named property has no name")),
            None => Vec::new(),
        };
        let values = property.values();
        // Validate the values before writing anything so an invalid property
        // is not partially written.
        let encoded = if tag.value_type() == Type::NULL {
            if !matches!(values, [] | [Value::Null]) {
                return Err(invalid("a Null property has no value"));
            }
            Vec::new()
        } else {
            if !tag.is_multi_valued() && values.len() != 1 {
                return Err(invalid("a single-valued property needs exactly one value"));
            }
            if values.len() as u64 > i32::MAX as u64 {
                return Err(invalid("too many property values"));
            }
            values
                .iter()
                .map(|v| encode(self.charset, tag.value_type(), v))
                .collect::<io::Result<Vec<_>>>()?
        };
        let length = encoded
            .iter()
            .try_fold(descriptor.len() as u64, |sum, value| {
                sum.checked_add(value.wire_length())
                    .ok_or_else(|| invalid("property size overflow"))
            })?;
        self.check_tag(tag, length)?;
        let tag = if let Some(name) = property.name() {
            Tag::new(self.stream.named_id(name)?, Type(tag.0 as u16))
        } else {
            tag
        };
        self.start(tag, &descriptor)?;
        for value in &encoded {
            self.emit(value)?;
        }
        self.current.as_mut().unwrap().values = encoded.len() as u32;
        Ok(())
    }

    /// Flushes scratch storage only, without completing the current property.
    ///
    /// # Errors
    ///
    /// Scratch flush errors are terminal.
    pub fn flush(&mut self) -> io::Result<()> {
        self.check()?;
        self.stream.flush()
    }

    /// Completes counts, emits the attribute, and returns its scratch buffer.
    ///
    /// # Errors
    ///
    /// Incomplete scalars and scratch/output errors leave the parent faulted.
    /// Owned scratch is dropped on error; use borrowed storage to retain it.
    pub fn finish(mut self) -> io::Result<B> {
        self.check()?;
        self.end_row()?;
        if self.table {
            self.stream.patch_u32(0, self.row_count)?;
        }
        self.stream.finish()
    }

    fn check(&self) -> io::Result<()> {
        if self.is_faulted() {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the TNEF property writer is faulted",
            ))
        } else {
            Ok(())
        }
    }

    fn reserve(&self, additional: u64) -> io::Result<()> {
        if additional > i32::MAX as u64 - self.stream.len() {
            Err(invalid(
                "TNEF attribute values cannot exceed i32::MAX bytes",
            ))
        } else {
            Ok(())
        }
    }

    fn check_value(&self) -> io::Result<&Current> {
        self.check()?;
        let current = self
            .current
            .as_ref()
            .ok_or_else(|| invalid("begin a property first"))?;
        if !current.tag.is_multi_valued() && current.values != 0 {
            return Err(invalid("the single-valued property already has a value"));
        }
        Ok(current)
    }

    fn check_complete(&self) -> io::Result<()> {
        if let Some(current) = &self.current
            && !current.tag.is_multi_valued()
            && current.tag.value_type() != Type::NULL
            && current.values != 1
        {
            return Err(invalid(
                "the single-valued property must have exactly one value",
            ));
        }
        Ok(())
    }

    fn check_tag(&self, tag: Tag, extra: u64) -> io::Result<()> {
        self.check()?;
        validate_type(tag)?;
        if self.count_offset.is_none() {
            return Err(invalid("begin a recipient row first"));
        }
        self.check_complete()?;
        increment(self.property_count)?;
        let header = if tag.is_multi_valued() || matches!(tag.value_type().width(), Width::Variable)
        {
            8
        } else {
            4
        };
        self.reserve(
            extra
                .checked_add(header)
                .ok_or_else(|| invalid("property size overflow"))?,
        )
    }

    fn start(&mut self, tag: Tag, descriptor: &[u8]) -> io::Result<()> {
        self.end_property()?;
        // MS-OXTNEF 2.1.3.5: a tag is the 16-bit type followed by the 16-bit ID.
        self.stream.write_all(&tag.0.to_le_bytes())?;
        self.stream.write_all(descriptor)?;
        let count_offset = if tag.is_multi_valued() {
            let offset = self.stream.len();
            self.stream.write_all(&[0; 4])?;
            Some(offset)
        } else {
            // A single variable-length value is also preceded by a count.
            if matches!(tag.value_type().width(), Width::Variable) {
                self.stream.write_all(&1u32.to_le_bytes())?;
            }
            None
        };
        self.current = Some(Current {
            tag,
            values: 0,
            count_offset,
        });
        self.property_count += 1;
        Ok(())
    }

    fn end_property(&mut self) -> io::Result<()> {
        self.check_complete()?;
        if let Some(current) = &self.current
            && let Some(offset) = current.count_offset
        {
            self.stream.patch_u32(offset, current.values)?;
        }
        // A Null property has no value.
        self.current = None;
        Ok(())
    }

    fn end_row(&mut self) -> io::Result<()> {
        self.end_property()?;
        if let Some(offset) = self.count_offset {
            self.stream.patch_u32(offset, self.property_count)?;
        }
        Ok(())
    }

    fn emit(&mut self, value: &Encoded<'_>) -> io::Result<()> {
        match value {
            Encoded::Fixed(bytes, length) => self.stream.write_all(&bytes[..*length]),
            Encoded::Variable(bytes, terminator) => {
                let length = bytes.len() as u64 + *terminator as u64;
                self.stream.write_all(&(length as u32).to_le_bytes())?;
                self.stream.write_all(bytes)?;
                self.stream.write_all(&[0; 2][..*terminator])?;
                self.stream.write_all(&[0; 3][..padding(length)])
            }
        }
    }
}

/// An append-only stream for a single Binary or Object property value.
///
/// Obtained from [`TnefPropertyWriter::open_value`]. Implements [`Write`], not
/// [`Read`] or [`Seek`]; lengths exclude the length prefix and DWORD padding.
/// `flush` flushes scratch only, not the output sink, and does not finish a value.
/// No I/O occurs on drop. Abandoning or forgetting the stream prevents successful
/// property/attribute completion, including for an empty multivalue.
///
/// ```
/// use std::io::Write;
/// use mimecraft::tnef::{TnefAttributeTag as A, TnefPropertyTag as P, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0)?;
/// let mut properties = writer.open_properties(A::MAPI_PROPERTIES)?;
/// properties.write_tag(P::ATTACH_DATA_BIN)?;
/// let mut value = properties.open_value()?;
/// value.write_all(b"binary data")?;
/// value.finish()?;
/// properties.finish()?;
/// writer.finish()?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// The property writer cannot advance while a value is borrowed:
///
/// ```compile_fail
/// use mimecraft::tnef::{TnefAttributeTag as A, TnefPropertyTag as P, TnefWriter};
/// let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
/// let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
/// properties.write_tag(P::RTF_COMPRESSED).unwrap();
/// let value = properties.open_value().unwrap();
/// properties.write_tag(P::SUBJECT_W).unwrap();
/// value.finish().unwrap();
/// ```
#[derive(Debug)]
#[must_use = "call finish() or the property writer remains faulted"]
pub struct TnefPropertyValueWriter<'p, 'a, W, B = MemoryBlockStream> {
    parent: &'p mut TnefPropertyWriter<'a, W, B>,
    offset: u64,
}

impl<W: Write, B: Read + Write + Seek> EmbeddedOutput for TnefPropertyWriter<'_, W, B> {
    fn write_embedded(&mut self, offset: u64, bytes: &[u8]) -> io::Result<usize> {
        TnefPropertyValueWriter {
            parent: self,
            offset,
        }
        .write(bytes)
    }

    fn flush_embedded(&mut self, offset: u64) -> io::Result<()> {
        TnefPropertyValueWriter {
            parent: self,
            offset,
        }
        .flush()
    }

    fn finish_embedded(&mut self, offset: u64) -> io::Result<()> {
        TnefPropertyValueWriter {
            parent: self,
            offset,
        }
        .finish()
    }
}

impl<W: Write, B: Read + Write + Seek> TnefPropertyValueWriter<'_, '_, W, B> {
    pub(super) fn patch(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        self.parent.stream.check_faulted()?;
        if offset > self.len() || bytes.len() as u64 > self.len() - offset {
            return Err(invalid("the patch is outside the property value"));
        }
        self.parent
            .stream
            .patch_bytes(self.offset + 4 + offset, bytes)
    }

    /// Number of payload bytes acknowledged by successful writes.
    pub fn len(&self) -> u64 {
        self.parent.stream.len() - self.offset - 4
    }

    /// Whether no payload bytes have been accepted.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether scratch I/O has failed, preventing further operations.
    pub fn is_faulted(&self) -> bool {
        self.parent.stream.is_faulted()
    }

    /// Patches the value length, pads to a DWORD, and completes one value.
    ///
    /// Does not flush scratch or emit the attribute. Finish the property writer
    /// and then its parent separately.
    ///
    /// # Errors
    ///
    /// An Object shorter than its 16-byte IID is rejected. Scratch seek/write
    /// errors are terminal. Any failure leaves the property writer blocked;
    /// borrowed attribute scratch can still be recovered with `into_inner`.
    pub fn finish(self) -> io::Result<()> {
        self.parent.stream.check_faulted()?;
        let length = self.len();
        if self.parent.current.as_ref().unwrap().tag.value_type() == Type::OBJECT && length < 16 {
            return Err(invalid(
                "an Object value must begin with a 16-byte interface IID",
            ));
        }
        // Complete a variable-length value written by a stream: patch its byte
        // length, add DWORD padding, and only then count the completed value.
        self.parent.stream.patch_u32(self.offset, length as u32)?;
        self.parent.stream.write_all(&[0; 3][..padding(length)])?;
        self.parent.current.as_mut().unwrap().values += 1;
        self.parent.value_open = false;
        Ok(())
    }
}

impl<W: Write, B: Read + Write + Seek> Write for TnefPropertyValueWriter<'_, '_, W, B> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.parent.stream.check_faulted()?;
        if input.is_empty() {
            return Ok(0);
        }
        // Reserve final padding as well as payload before touching scratch, so
        // a successful write cannot leave an unfinishable over-limit value.
        self.parent
            .reserve(value_growth(self.len(), input.len() as u64)?)?;
        self.parent.stream.write(input)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.parent.stream.flush()
    }
}

fn value_growth(length: u64, additional: u64) -> io::Result<u64> {
    let total = length
        .checked_add(additional)
        .filter(|&total| total <= i32::MAX as u64)
        .ok_or_else(|| invalid("the property value exceeds i32::MAX bytes"))?;
    Ok(additional + padding(total) as u64)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn increment(count: u32) -> io::Result<u32> {
    if count >= i32::MAX as u32 {
        Err(invalid("the TNEF count exceeds i32::MAX"))
    } else {
        Ok(count + 1)
    }
}
fn padding(length: u64) -> usize {
    ((4 - (length & 3)) & 3) as usize
}

fn validate_type(tag: Tag) -> io::Result<()> {
    let kind = tag.value_type();
    if matches!(kind.width(), Width::Unsupported)
        || (tag.is_multi_valued()
            && matches!(
                kind,
                Type::NULL | Type::ERROR | Type::BOOLEAN | Type::OBJECT
            ))
    {
        return Err(invalid("the property type cannot be written"));
    }
    Ok(())
}

fn name_bytes(name: &TnefNameId) -> io::Result<Vec<u8>> {
    let mut bytes = name.guid.to_vec();
    match &name.name {
        TnefName::Id(id) => {
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&id.to_le_bytes());
        }
        TnefName::String(text) => {
            // MS-OXTNEF 2.1.3.5: the name is a NUL-terminated UTF-16LE
            // string preceded by its byte length (including the terminator)
            // and padded to a multiple of four bytes.
            let length = unicode_length(text)?;
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.extend_from_slice(&(length as u32).to_le_bytes());
            bytes.extend(text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
            bytes.extend_from_slice(&[0; 3][..padding(length)]);
        }
        TnefName::Unknown(_) => return Err(invalid("the property name kind is unsupported")),
    }
    Ok(bytes)
}

enum Encoded<'a> {
    Fixed([u8; 16], usize),
    Variable(Cow<'a, [u8]>, usize),
}

impl Encoded<'_> {
    fn wire_length(&self) -> u64 {
        match self {
            Self::Fixed(_, length) => *length as u64,
            Self::Variable(bytes, terminator) => {
                let length = bytes.len() as u64 + *terminator as u64;
                4 + length + padding(length) as u64
            }
        }
    }
}

fn unicode_length(text: &str) -> io::Result<u64> {
    let length = (text.encode_utf16().count() as u64 + 1) * 2;
    if length > i32::MAX as u64 {
        Err(invalid("the UTF-16 value is too large"))
    } else {
        Ok(length)
    }
}

fn encode(charset: Charset, kind: Type, value: &Value) -> io::Result<Encoded<'_>> {
    let mut bytes = [0; 16];
    let length = match (kind, value) {
        (Type::I2, Value::I16(value)) => {
            // MS-OXTNEF pads 16-bit values to four bytes, with zero padding.
            bytes[..2].copy_from_slice(&value.to_le_bytes());
            4
        }
        (Type::LONG, Value::I32(value)) => {
            bytes[..4].copy_from_slice(&value.to_le_bytes());
            4
        }
        (Type::ERROR, Value::Error(value)) => {
            bytes[..4].copy_from_slice(&value.to_le_bytes());
            4
        }
        (Type::R4, Value::F32(value)) => {
            bytes[..4].copy_from_slice(&value.to_le_bytes());
            4
        }
        (Type::DOUBLE, Value::F64(value)) => {
            bytes[..8].copy_from_slice(&value.to_le_bytes());
            8
        }
        (Type::I8, Value::I64(value)) | (Type::CURRENCY, Value::Currency(value)) => {
            // Currency is already an exact integer in units of 1/10000.
            bytes[..8].copy_from_slice(&value.to_le_bytes());
            8
        }
        (Type::APP_TIME, Value::AppTime(value)) => {
            super::TnefDateTime::from_oa_date(*value)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            bytes[..8].copy_from_slice(&value.to_le_bytes());
            8
        }
        (Type::SYS_TIME, Value::SysTime(value)) => {
            // MS-OXCDATA PtypTime is a FILETIME: 100-nanosecond intervals
            // since January 1, 1601 UTC.
            super::TnefDateTime::from_file_time(*value)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            bytes[..8].copy_from_slice(&value.to_le_bytes());
            8
        }
        (Type::BOOLEAN, Value::Bool(value)) => {
            bytes[0] = u8::from(*value);
            4
        }
        (Type::CLASS_ID, Value::Guid(value)) => {
            bytes.copy_from_slice(value);
            16
        }
        (Type::BINARY, Value::Binary(value)) => {
            return Ok(Encoded::Variable(Cow::Borrowed(value), 0));
        }
        (Type::OBJECT, Value::Object(value)) => {
            if value.len() < 16 {
                return Err(invalid("an Object value needs a 16-byte interface IID"));
            }
            return Ok(Encoded::Variable(Cow::Borrowed(value), 0));
        }
        (Type::STRING8, Value::Text(text)) => {
            let bytes = charset
                .encode(text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            return Ok(Encoded::Variable(bytes, 1));
        }
        (Type::UNICODE, Value::Text(text)) => {
            unicode_length(text)?;
            let bytes = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            return Ok(Encoded::Variable(Cow::Owned(bytes), 2));
        }
        _ => return Err(invalid("the value is not the property's native type")),
    };
    Ok(Encoded::Fixed(bytes, length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tnef::{TnefAttributeTag as A, TnefPropertyInfo, TnefWriter};

    #[test]
    fn streamed_value_growth_reserves_padding_and_checks_exact_limits() {
        for length in 0..8 {
            for additional in 0..8 {
                assert_eq!(
                    value_growth(length, additional).unwrap(),
                    additional + padding(length + additional) as u64
                );
            }
        }
        let max = i32::MAX as u64;
        assert_eq!(value_growth(0, max).unwrap(), max + 1);
        assert_eq!(value_growth(max - 1, 1).unwrap(), 2);
        for (length, additional) in [(0, max + 1), (max, 1), (u64::MAX, 1), (1, u64::MAX)] {
            assert_eq!(
                value_growth(length, additional).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
        properties.write_tag(Tag::ATTACH_DATA_BIN).unwrap();
        let value = properties.open_value().unwrap();
        let capacity = max - value.parent.stream.len();
        for additional in (capacity - 3)..=capacity {
            let growth = value_growth(0, additional).unwrap();
            assert_eq!(
                value.parent.reserve(growth).is_ok(),
                additional == capacity - 3
            );
        }
        assert_eq!(value.len(), 0);
        value.finish().unwrap();
        properties.finish().unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn streamed_value_count_exhaustion_is_recoverable_before_the_placeholder() {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
        properties
            .write_tag(Tag::new(
                0x6000,
                Type(Type::BINARY.0 | Type::MULTI_VALUED.0),
            ))
            .unwrap();
        properties.current.as_mut().unwrap().values = i32::MAX as u32;
        let before = properties.get_ref().to_vec().unwrap();
        assert_eq!(
            properties.open_value().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            properties
                .open_rtf_compressed(RtfCompressionMode::Compressed)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!properties.is_faulted());
        assert_eq!(properties.get_ref().to_vec().unwrap(), before);
        properties.current.as_mut().unwrap().values = 0;
        properties.open_value().unwrap().finish().unwrap();
        assert_eq!(properties.current.as_ref().unwrap().values, 1);
        properties.finish().unwrap();
        writer.finish().unwrap();
    }

    fn snapshot(tag: Tag, name: Option<TnefNameId>, values: Vec<Value>) -> TnefProperty {
        TnefProperty::new(
            TnefPropertyInfo {
                tag,
                name,
                offset: 0,
            },
            values,
            crate::charsets::from_codepage(1252).unwrap(),
        )
    }

    #[test]
    fn invalid_snapshots_are_preflighted_without_partial_properties_or_named_ids() {
        let name = TnefNameId {
            guid: [0; 16],
            name: TnefName::String("test".into()),
        };
        let cases = [
            snapshot(Tag::new(0x6000, Type::NULL), None, vec![Value::I32(1)]),
            snapshot(Tag::SUBJECT_W, None, vec![]),
            snapshot(Tag::IMPORTANCE, None, vec![Value::I32(1), Value::I32(2)]),
            snapshot(Tag::new(0x8000, Type::LONG), None, vec![Value::I32(1)]),
            snapshot(
                Tag::new(0x8000, Type(Type::STRING8.0 | Type::MULTI_VALUED.0)),
                Some(name.clone()),
                vec![Value::Text("valid".into()), Value::Text("\u{1f600}".into())],
            ),
            snapshot(
                Tag::new(0x6000, Type::OBJECT),
                None,
                vec![Value::Object(vec![0; 15])],
            ),
            snapshot(
                Tag::new(0x6000, Type::SYS_TIME),
                None,
                vec![Value::SysTime(-1)],
            ),
            snapshot(
                Tag::new(0x6000, Type::LONG),
                None,
                vec![Value::Text("1".into())],
            ),
        ];
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
        for value in &cases {
            assert_eq!(
                properties.write_property(value).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(properties.property_count(), 0);
            assert_eq!(properties.get_ref().to_vec().unwrap(), [0; 4]);
        }
        properties.write_named_tag(&name, Type::NULL).unwrap();
        let bytes = properties.finish().unwrap().to_vec().unwrap();
        assert_eq!(&bytes[4..8], &Tag::new(0x8000, Type::NULL).0.to_le_bytes());
        writer.finish().unwrap();
    }

    #[test]
    fn property_value_and_row_count_limits_preserve_the_buffer() {
        assert_eq!(increment(i32::MAX as u32 - 1).unwrap(), i32::MAX as u32);
        assert!(increment(i32::MAX as u32).is_err());
        assert!(increment(u32::MAX).is_err());
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer.open_properties(A::RECIPIENT_TABLE).unwrap();
        properties.row_count = i32::MAX as u32;
        assert!(properties.begin_row().is_err());
        properties.row_count = 0;
        properties.begin_row().unwrap();
        properties.property_count = i32::MAX as u32;
        assert!(properties.write_tag(Tag::IMPORTANCE).is_err());
        properties.property_count = 0;
        properties
            .write_tag(Tag::new(0x6000, Type(Type::LONG.0 | Type::MULTI_VALUED.0)))
            .unwrap();
        properties.current.as_mut().unwrap().values = i32::MAX as u32;
        let before = properties.get_ref().to_vec().unwrap();
        assert!(properties.write_value(&Value::I32(1)).is_err());
        assert!(properties.reserve(i32::MAX as u64).is_err());
        assert!(
            properties
                .reserve(i32::MAX as u64 - properties.stream.len())
                .is_ok()
        );
        assert!(properties.reserve(u64::MAX).is_err());
        assert_eq!(properties.get_ref().to_vec().unwrap(), before);
        properties.current.as_mut().unwrap().values = 0;
        properties.finish().unwrap();
        writer.finish().unwrap();
    }
}

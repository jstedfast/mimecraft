// Adapted from MimeKit's TnefPropertyReader.cs and TnefReaderStream.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::embedded::{EmbeddedInput, TnefEmbeddedReader, TnefEmbeddedSource};
use super::{Logger, TnefReader};
use crate::tnef::{
    TnefAttributeLevel, TnefAttributeTag, TnefComplianceViolation as Violation, TnefName,
    TnefNameId, TnefProperty, TnefPropertyInfo, TnefPropertySet, TnefPropertyTag as Tag,
    TnefPropertyType as Type, TnefReadError as Error, TnefValue, property_types::Width,
};
use std::io::{self, Read};

/// Exclusively borrowed, forward-only MAPI property and recipient-row reader.
///
/// Select a recipient row before reading its properties. Selecting a property
/// also selects its first value, unless its value count is zero. Advancing
/// skips unread values and DWORD padding without crossing the attribute.
///
/// Native fixed-width values are cached and repeatable. Variable values can be
/// claimed once, by collection or a bounded stream. The parent's owned-byte and
/// diagnostic budgets are shared, including bytes allocated for property names.
/// An oversized name stops this property reader with a limit error; the parent
/// remains usable for skipping the attribute. Oversized value collection leaves
/// the value unclaimed, permitting streaming instead.
///
/// No I/O occurs on drop. Embedded readers share limits and diagnostics;
/// numeric coercions are not yet implemented. Invalid dates are diagnosed while loading fixed-width values;
/// native date values retain their original wire numbers.
/// Values remain provisional until parent advance verifies the checksum.
///
/// ```
/// use mimecraft::tnef::{TnefReader, TnefValue};
/// # let input = b"\x78\x9f\x3e\x22\0\0\x01\x03\x90\x06\0\x0c\0\0\0\x01\0\0\0\x03\0\x17\0\x07\0\0\0\x22\0";
/// let mut reader = TnefReader::new(input.as_slice());
/// assert!(reader.read_next()?);
/// {
///     let mut properties = reader.open_properties()?;
///     assert!(properties.read_next_property()?);
///     assert_eq!(properties.property().unwrap().tag.id(), 0x0017);
///     assert_eq!(properties.read_value()?, TnefValue::I32(7));
///     assert!(!properties.read_next_property()?);
/// }
/// // Advancing the parent verifies the attribute checksum.
/// assert!(!reader.read_next()?);
/// # Ok::<(), mimecraft::tnef::TnefReadError>(())
/// ```
pub struct TnefPropertyReader<'a, R> {
    reader: &'a mut TnefReader<R>,
    logger: Logger<'a>,
    table: bool,
    row_count: Option<usize>,
    row_index: usize,
    in_row: bool,
    property_count: Option<usize>,
    property_index: usize,
    property: Option<TnefPropertyInfo>,
    stopped: bool,
    value_count: usize,
    value_index: usize,
    has_value: bool,
    consumed: bool,
    data_end: u64,
    value_end: u64,
    fixed: Option<TnefValue>,
    object_iid: [u8; 16],
}

impl<'a, R: Read> TnefPropertyReader<'a, R> {
    pub(super) fn new(reader: &'a mut TnefReader<R>, logger: Logger<'a>, table: bool) -> Self {
        Self {
            reader,
            logger,
            table,
            row_count: None,
            row_index: 0,
            in_row: false,
            property_count: None,
            property_index: 0,
            property: None,
            stopped: false,
            value_count: 0,
            value_index: 0,
            has_value: false,
            consumed: false,
            data_end: 0,
            value_end: 0,
            fixed: None,
            object_iid: [0; 16],
        }
    }

    /// Whether this attribute contains recipient rows.
    pub fn is_table(&self) -> bool {
        self.table
    }
    /// Declared/recovered row count, zero before the first row read.
    pub fn row_count(&self) -> usize {
        self.row_count.unwrap_or(0)
    }
    /// Declared/recovered property count for the current row or attribute.
    pub fn property_count(&self) -> usize {
        self.property_count.unwrap_or(0)
    }
    /// Current metadata, absent before selection and after stopping.
    pub fn property(&self) -> Option<&TnefPropertyInfo> {
        self.property.as_ref()
    }
    /// Declared/recovered count of values in the current property.
    pub fn value_count(&self) -> usize {
        self.value_count
    }
    /// Whether a value is currently selected.
    pub fn has_value(&self) -> bool {
        self.has_value
    }
    /// Whether a variable value has already been claimed.
    pub fn is_value_consumed(&self) -> bool {
        self.consumed
    }

    /// Whether the current AttachData value is an embedded message.
    /// Recognition does not claim bytes or validate the nested TNEF signature.
    pub fn is_embedded_message(&self) -> bool {
        let Some(property) = &self.property else {
            return false;
        };
        if !self.has_value || property.tag.id() != 0x3701 {
            return false;
        }
        let kind = property.tag.value_type();
        // MS-OXTNEF prefixes PtypObject with its interface IID. AttachMethod
        // (0x3705) usually follows AttachDataObject (0x3701), so the IID is the
        // only reliable indicator when properties are emitted in ID order.
        if kind == Type::OBJECT && self.object_iid != [0; 16] {
            return self.object_iid == [7, 3, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];
        }
        self.reader.attach_method == 5 && matches!(kind, Type::OBJECT | Type::BINARY)
    }

    /// Claims a bounded embedded message without collecting its payload.
    ///
    /// The child exclusively borrows this reader, inherits its logger and
    /// current charset, and shares the root's byte/report budgets. Its first
    /// advance skips the 16-byte interface IID. Opening/dropping does no I/O.
    ///
    /// # Errors
    ///
    /// Rejects absent, nonembedded, claimed, or faulted values. Unlike MimeKit's
    /// empty-reader fallback, exceeding the depth limit reports `NestingTooDeep`
    /// and returns `NestingLimitExceeded`, leaving raw streaming/skipping usable.
    pub fn open_embedded_message(&mut self) -> Result<TnefEmbeddedReader<'_, 'a>, Error> {
        self.check_variable()?;
        if !self.is_embedded_message() {
            return Err(Error::NotEmbeddedMessage);
        }
        if self.reader.depth >= self.reader.options.max_nesting_depth {
            self.run(|properties| {
                properties.log(Violation::NestingTooDeep, properties.reader.position)
            })?;
            return Err(Error::NestingLimitExceeded);
        }
        let depth = self.reader.depth + 1;
        let position = self.reader.position;
        let options = self.reader.options;
        let charset = self.reader.charset;
        let codepage = self.reader.codepage;
        // Only the root's budget is used; siblings and deeper descendants must
        // not gain fresh allowances by opening another reader.
        let shared = self.reader.shared_state();
        let logger = self.logger.share();
        self.consumed = true;
        let mut reader = TnefReader::with_options(TnefEmbeddedSource::new(self), &options)?;
        reader.shared = Some(shared);
        reader.depth = depth;
        reader.position = position;
        reader.skip_iid = true;
        reader.charset = charset;
        reader.codepage = codepage;
        Ok(TnefEmbeddedReader { reader, logger })
    }
    /// Whether malformed structure made subsequent property boundaries unknowable.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }
    /// Consumed position in the parent TNEF stream.
    pub fn position(&self) -> u64 {
        self.reader.position
    }

    /// Selects the next row, skipping remaining properties of the current row.
    /// Returns false for non-table attributes.
    ///
    /// # Errors
    ///
    /// Source/logger errors fault the parent. An oversized named-property
    /// descriptor returns a limit error and stops property traversal.
    pub fn read_next_row(&mut self) -> Result<bool, Error> {
        self.run(Self::next_row)
    }

    /// Selects the next property and its first value. For tables a row must
    /// first be selected. Unread values of the previous property are skipped.
    ///
    /// # Errors
    ///
    /// Source/logger failures or a named-property byte-limit failure.
    pub fn read_next_property(&mut self) -> Result<bool, Error> {
        self.run(Self::next_property)
    }

    /// Skips the current value and selects the next, including for a malformed
    /// nominally single-valued property advertising more than one value.
    ///
    /// # Errors
    ///
    /// Source/logger errors fault the parent.
    pub fn read_next_value(&mut self) -> Result<bool, Error> {
        self.run(Self::next_value)
    }

    /// Collects the current property into an owned native-value snapshot.
    ///
    /// Call before reading a variable value or advancing past the first value.
    /// Cached fixed-width reads do not prevent collection. As in MimeKit,
    /// nominally single-valued properties collect only the first value even if
    /// their malformed count advertises more; parent property advance skips
    /// those extra values. Multivalued properties collect all available values.
    ///
    /// Allocation grows with values actually read, never an unchecked count.
    /// The byte budgets cover wire names and variable payloads, not collection
    /// storage, cloned metadata, or fixed-width values. Malformed/truncated input
    /// can return a partial snapshot, with diagnostics through the supplied logger.
    ///
    /// # Errors
    ///
    /// Rejects absent, already-consumed, or advanced properties. Limit failures
    /// return an error rather than an empty-success property. Earlier reads are
    /// not rolled back or refunded; the oversized value remains unclaimed for
    /// streaming/skipping. Source/logger errors fault the parent.
    pub fn read_property(&mut self) -> Result<TnefProperty, Error> {
        if self.reader.is_faulted() {
            return Err(Error::Faulted);
        }
        let info = self.property.as_ref().ok_or(Error::NoProperty)?;
        if self.value_index > 1 || self.consumed || (self.value_count > 0 && !self.has_value) {
            return Err(Error::ValueAlreadyRead);
        }
        let info = info.clone();
        // A bogus value count must not force a huge allocation. Start with
        // space only for the selected value and grow as values actually arrive.
        let mut values = Vec::with_capacity(usize::from(self.has_value));
        if self.has_value {
            loop {
                values.push(self.read_value()?);
                if !info.tag.is_multi_valued()
                    || values.len() == self.value_count
                    || !self.read_next_value()?
                {
                    break;
                }
            }
        }
        Ok(TnefProperty::new(info, values))
    }

    /// Collects all properties following the current property, or all
    /// properties if none has yet been selected. For a table, collects only
    /// the selected row; without a selected row the result is empty.
    ///
    /// The current property's remaining values are skipped, not included.
    /// Binary/object values are collected; use bounded streams for large data.
    ///
    /// # Errors
    ///
    /// Shares [`read_property`](Self::read_property) errors. Collection is not
    /// transactional: an error discards the partially built set, without
    /// rewinding the cursor or refunding bytes already collected.
    pub fn read_property_set(&mut self) -> Result<TnefPropertySet, Error> {
        let mut properties = Vec::new();
        while self.read_next_property()? {
            properties.push(self.read_property()?);
        }
        Ok(properties.into())
    }

    /// Collects all remaining recipient rows into owned property sets.
    ///
    /// Unread properties of the current row are skipped. Non-table attributes
    /// return an empty vector without advancing. Rows grow as they are read;
    /// declared counts are never used for upfront allocation.
    ///
    /// # Errors
    ///
    /// Shares the nontransactional collection and failure behavior of
    /// [`read_property_set`](Self::read_property_set).
    pub fn read_rows_as_property_sets(&mut self) -> Result<Vec<TnefPropertySet>, Error> {
        let mut rows = Vec::new();
        while self.read_next_row()? {
            rows.push(self.read_property_set()?);
        }
        Ok(rows)
    }

    /// Reads the selected AppTime/SysTime as a checked, repeatable date.
    /// The raw value remains available through [`read_value`](Self::read_value).
    ///
    /// # Errors
    ///
    /// Rejects absent values and other property types. Invalid dates return an
    /// explicit error without faulting the reader or repeating the diagnostic
    /// already emitted during value selection.
    pub fn read_value_datetime(&self) -> Result<crate::tnef::TnefDateTime, Error> {
        let tag = self.check_value()?;
        if !matches!(tag.value_type(), Type::APP_TIME | Type::SYS_TIME) {
            return Err(Error::InvalidPropertyType(tag));
        }
        self.fixed
            .as_ref()
            .unwrap()
            .to_datetime()
            .map_err(Error::InvalidDate)
    }

    /// Returns a cached fixed value, or collects and decodes a variable value.
    /// Currency and dates retain exact wire representations; no coercion occurs.
    ///
    /// # Errors
    ///
    /// Requires a selected value, with variable values still unclaimed.
    /// Owned-value limits are recoverable; source/logger failures are terminal.
    pub fn read_value(&mut self) -> Result<TnefValue, Error> {
        let tag = self.check_value()?;
        if let Some(value) = &self.fixed {
            return Ok(value.clone());
        }
        let bytes = self.read_value_bytes()?;
        Ok(match tag.value_type() {
            Type::STRING8 => {
                let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
                TnefValue::Text(
                    self.reader
                        .charset
                        .decode_lossy(&bytes[..end])
                        .0
                        .into_owned(),
                )
            }
            Type::UNICODE => TnefValue::Text(decode_unicode(&bytes)),
            Type::OBJECT => TnefValue::Object(bytes),
            _ => TnefValue::Binary(bytes),
        })
    }

    /// Collects the raw bytes of a string, binary, or object value, without
    /// its DWORD padding. Object interface GUID prefixes are retained.
    /// Physical truncation returns partial bytes and sets the parent's
    /// [`is_truncated`](TnefReader::is_truncated) flag.
    ///
    /// # Errors
    ///
    /// Rejects fixed-width or already-claimed values. A limit error leaves the
    /// value unclaimed; source/logger errors fault the parent.
    pub fn read_value_bytes(&mut self) -> Result<Vec<u8>, Error> {
        self.check_variable()?;
        let bytes = self
            .reader
            .collect_bytes(self.data_end - self.reader.position, &mut self.logger)?;
        self.consumed = true;
        Ok(bytes)
    }

    /// Opens a borrowed raw variable-value stream. Padding and following
    /// values cannot be consumed through it. Dropping performs no I/O.
    ///
    /// # Errors
    ///
    /// Rejects absent, fixed-width, already-claimed, or faulted values.
    pub fn open_value(&mut self) -> Result<TnefPropertyValueReader<'_, 'a, R>, Error> {
        self.check_variable()?;
        self.consumed = true;
        Ok(TnefPropertyValueReader { properties: self })
    }

    fn check_value(&self) -> Result<Tag, Error> {
        if self.reader.is_faulted() {
            return Err(Error::Faulted);
        }
        if !self.has_value {
            return Err(Error::NoPropertyValue);
        }
        Ok(self.property.as_ref().unwrap().tag)
    }

    fn check_variable(&self) -> Result<(), Error> {
        let tag = self.check_value()?;
        if !matches!(tag.value_type().width(), Width::Variable) {
            return Err(Error::InvalidPropertyType(tag));
        }
        if self.consumed {
            return Err(Error::ValueAlreadyRead);
        }
        Ok(())
    }

    fn run<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if self.reader.is_faulted() {
            return Err(Error::Faulted);
        }
        let result = operation(self);
        if matches!(&result, Err(Error::Io(_) | Error::PositionOverflow)) {
            self.reader.fault();
        }
        result
    }

    fn log(&mut self, violation: Violation, offset: u64) -> Result<(), Error> {
        self.reader.log(&mut self.logger, violation, offset)
    }

    fn stop(&mut self) {
        // Once an attribute's structure cannot be trusted, there is no way to
        // locate the properties that follow.
        self.stopped = true;
        self.property = None;
        self.has_value = false;
        self.in_row = false;
    }

    fn block(&mut self, count: usize, violation: Violation) -> Result<Option<[u8; 20]>, Error> {
        if self.reader.remaining < count as u64 {
            self.log(violation, self.reader.position)?;
            self.stop();
            return Ok(None);
        }
        if !self.reader.fill(count)? {
            self.reader.truncated(&mut self.logger)?;
            self.stop();
            return Ok(None);
        }
        let mut bytes = [0; 20];
        bytes[..count]
            .copy_from_slice(&self.reader.input[self.reader.index..self.reader.index + count]);
        self.reader.take_value(count)?;
        Ok(Some(bytes))
    }

    fn count(&mut self, violation: Violation) -> Result<Option<usize>, Error> {
        self.reader.property_tag = None;
        let offset = self.reader.position;
        let Some(bytes) = self.block(4, violation)? else {
            return Ok(None);
        };
        let count = i32::from_le_bytes(bytes[..4].try_into().unwrap());
        // Every row begins with a DWORD property count, and every property
        // begins with a DWORD tag. Neither count can exceed remaining / 4.
        let max = (self.reader.remaining / 4) as usize;
        if count < 0 || count as usize > max {
            self.log(violation, offset)?;
        }
        Ok(Some((count.max(0) as usize).min(max)))
    }

    fn skip_to(&mut self, end: u64) -> Result<bool, Error> {
        if self.reader.truncated {
            self.stop();
            return Ok(false);
        }
        while self.reader.position < end {
            if !self.reader.fill(1)? {
                self.reader.truncated(&mut self.logger)?;
                self.stop();
                return Ok(false);
            }
            let count =
                (self.reader.end - self.reader.index).min((end - self.reader.position) as usize);
            self.reader.take_value(count)?;
        }
        Ok(true)
    }

    fn next_row(&mut self) -> Result<bool, Error> {
        if !self.table || self.stopped {
            return Ok(false);
        }
        if self.in_row {
            while self.next_property()? {
                // Skip the remaining properties of the current row.
            }
            if self.stopped {
                return Ok(false);
            }
        } else if self.row_count.is_none() {
            self.row_count = self.count(Violation::InvalidRowCount)?;
            if self.row_count.is_none() {
                return Ok(false);
            }
        }
        self.in_row = false;
        if self.row_index >= self.row_count.unwrap() {
            return Ok(false);
        }
        self.row_index += 1;
        self.property_index = 0;
        self.property_count = self.count(Violation::InvalidPropertyCount)?;
        self.in_row = self.property_count.is_some();
        Ok(self.in_row)
    }

    fn next_property(&mut self) -> Result<bool, Error> {
        if self.stopped {
            return Ok(false);
        }
        while self.next_value()? {
            // Skip the remaining values of the current property.
        }
        self.property = None;
        self.reader.property_tag = None;
        if self.stopped || (self.table && !self.in_row) {
            return Ok(false);
        }
        if self.property_count.is_none() {
            self.property_count = self.count(Violation::InvalidPropertyCount)?;
            if self.property_count.is_none() {
                return Ok(false);
            }
        }
        if self.property_index >= self.property_count.unwrap() {
            return Ok(false);
        }
        self.property_index += 1;
        let offset = self.reader.position;
        let Some(bytes) = self.block(4, Violation::InvalidPropertyLength)? else {
            return Ok(false);
        };
        let tag = Tag(u32::from_le_bytes(bytes[..4].try_into().unwrap()));
        self.reader.property_tag = Some(tag);
        self.property = Some(TnefPropertyInfo {
            tag,
            name: None,
            offset,
        });
        self.value_count = 0;
        self.value_index = 0;
        self.has_value = false;
        self.fixed = None;
        self.consumed = false;
        if tag.is_named() && !self.read_name(offset)? {
            return Ok(false);
        }
        let width = tag.value_type().width();
        if matches!(width, Width::Unsupported) {
            // The width of an unknown type is unknowable; later properties
            // cannot be located safely.
            self.log(Violation::UnsupportedPropertyType, offset)?;
            self.stop();
            return Ok(false);
        }
        self.value_count = 1;
        if tag.is_multi_valued() || matches!(width, Width::Variable) {
            let offset = self.reader.position;
            let Some(bytes) = self.block(4, Violation::InvalidPropertyLength)? else {
                return Ok(false);
            };
            let count = i32::from_le_bytes(bytes[..4].try_into().unwrap());
            // Variable values each begin with a DWORD length.
            let size = match width {
                Width::Fixed(n) if n > 0 => n,
                _ => 4,
            };
            let max = (self.reader.remaining / size as u64) as usize;
            if matches!(width, Width::Fixed(0)) {
                // Multivalued Null is invalid and occupies no bytes, so input
                // cannot bound its count. Ignore its values, preserving alignment.
                if count != 0 {
                    self.log(Violation::InvalidValueCount, offset)?;
                }
                self.value_count = 0;
            } else {
                let invalid = if tag.is_multi_valued() {
                    count < 0 || count as usize > max
                } else {
                    count != 1
                };
                // Honor non-one single-value counts (within bounds) so following
                // properties remain aligned.
                if invalid {
                    self.log(Violation::InvalidValueCount, offset)?;
                }
                self.value_count = (count.max(0) as usize).min(max);
            }
        }
        Ok(self.value_count == 0 || self.position_value()?)
    }

    fn read_name(&mut self, property_offset: u64) -> Result<bool, Error> {
        let Some(bytes) = self.block(20, Violation::InvalidPropertyLength)? else {
            return Ok(false);
        };
        let guid = bytes[..16].try_into().unwrap();
        let kind = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let name = match kind {
            0 => {
                let Some(bytes) = self.block(4, Violation::InvalidPropertyLength)? else {
                    return Ok(false);
                };
                TnefName::Id(i32::from_le_bytes(bytes[..4].try_into().unwrap()))
            }
            1 => {
                let offset = self.reader.position;
                let Some(bytes) = self.block(4, Violation::InvalidPropertyLength)? else {
                    return Ok(false);
                };
                let length = i32::from_le_bytes(bytes[..4].try_into().unwrap());
                if length < 0 || length as u64 > self.reader.remaining {
                    self.log(Violation::InvalidPropertyLength, offset)?;
                    self.stop();
                    return Ok(false);
                }
                let bytes = match self.reader.collect_bytes(length as u64, &mut self.logger) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        self.stop();
                        return Err(error);
                    }
                };
                if bytes.len() != length as usize {
                    self.stop();
                    return Ok(false);
                }
                let padding = (4 - (length & 3)) & 3;
                if padding as u64 > self.reader.remaining {
                    self.log(Violation::InvalidPropertyLength, self.reader.position)?;
                }
                let end = self
                    .reader
                    .position
                    .checked_add((padding as u64).min(self.reader.remaining))
                    .ok_or(Error::PositionOverflow)?;
                if !self.skip_to(end)? {
                    return Ok(false);
                }
                TnefName::String(decode_unicode(&bytes))
            }
            kind => {
                self.log(Violation::InvalidNamedPropertyKind, property_offset)?;
                TnefName::Unknown(kind)
            }
        };
        self.property.as_mut().unwrap().name = Some(TnefNameId { guid, name });
        Ok(true)
    }

    fn next_value(&mut self) -> Result<bool, Error> {
        if self.property.is_none() || self.stopped {
            return Ok(false);
        }
        if self.has_value {
            self.has_value = false;
            if !self.skip_to(self.value_end)? {
                return Ok(false);
            }
        }
        if self.value_index >= self.value_count {
            return Ok(false);
        }
        self.position_value()
    }

    fn position_value(&mut self) -> Result<bool, Error> {
        self.value_index += 1;
        self.consumed = false;
        self.has_value = false;
        self.fixed = None;
        self.object_iid = [0; 16];
        let tag = self.property.as_ref().unwrap().tag;
        match tag.value_type().width() {
            Width::Fixed(width) => {
                // The largest fixed value is a sixteen-byte GUID.
                let offset = self.reader.position;
                let Some(bytes) = self.block(width, Violation::InvalidPropertyLength)? else {
                    return Ok(false);
                };
                self.fixed = Some(fixed_value(tag.value_type(), &bytes));
                if matches!(tag.value_type(), Type::APP_TIME | Type::SYS_TIME)
                    && self.fixed.as_ref().unwrap().to_datetime().is_err()
                {
                    self.log(Violation::InvalidDate, offset)?;
                }
                self.data_end = self.reader.position;
                self.value_end = self.data_end;
                if tag.id() == 0x3705 && tag.value_type() == Type::LONG {
                    self.reader.attach_method = i32::from_le_bytes(bytes[..4].try_into().unwrap());
                }
                if tag.id() == 0x3fde
                    && tag.value_type() == Type::LONG
                    && !tag.is_multi_valued()
                    && self.reader.attribute.unwrap().level == TnefAttributeLevel::Message
                    && self.reader.attribute.unwrap().tag == TnefAttributeTag::MAPI_PROPERTIES
                {
                    let page = i32::from_le_bytes(bytes[..4].try_into().unwrap());
                    // InternetCodepage is a fallback only without a usable OEM
                    // codepage. UTF-16/32 contain embedded zero bytes. As in
                    // MimeKit, unsupported fallback hints retain the current
                    // charset without a diagnostic (unlike OEM declarations).
                    if !self.reader.has_oem_codepage
                        && page > 0
                        && !matches!(page, 1200 | 1201 | 12000 | 12001)
                        && let Some(charset) = crate::charsets::from_codepage(page as u32)
                    {
                        self.reader.codepage = page as u32;
                        self.reader.charset = charset;
                    }
                }
            }
            Width::Variable => {
                let offset = self.reader.position;
                let Some(bytes) = self.block(4, Violation::InvalidPropertyLength)? else {
                    return Ok(false);
                };
                let length = i32::from_le_bytes(bytes[..4].try_into().unwrap());
                if length < 0 || length as u64 > self.reader.remaining {
                    self.log(Violation::InvalidPropertyLength, offset)?;
                    self.stop();
                    return Ok(false);
                }
                self.data_end = self
                    .reader
                    .position
                    .checked_add(length as u64)
                    .ok_or(Error::PositionOverflow)?;
                let padding = ((4 - (length & 3)) & 3) as u64;
                let available = self.reader.remaining - length as u64;
                if padding > available {
                    self.log(Violation::InvalidPropertyLength, self.data_end)?;
                }
                self.value_end = self
                    .data_end
                    .checked_add(padding.min(available))
                    .ok_or(Error::PositionOverflow)?;
                // Peek without claiming the prefix. If the source ends before
                // the IID, truncation is reported when the value is read/skipped.
                if tag.value_type() == Type::OBJECT && length >= 16 && self.reader.fill(16)? {
                    self.object_iid.copy_from_slice(
                        &self.reader.input[self.reader.index..self.reader.index + 16],
                    );
                }
            }
            Width::Unsupported => unreachable!("property type checked before selecting values"),
        }
        self.has_value = true;
        Ok(true)
    }
}

impl<R> Drop for TnefPropertyReader<'_, R> {
    fn drop(&mut self) {
        self.reader.property_tag = None;
    }
}

/// A raw variable property value, excluding its DWORD padding.
pub struct TnefPropertyValueReader<'p, 'r, R> {
    properties: &'p mut TnefPropertyReader<'r, R>,
}

impl<R: Read> Read for TnefPropertyValueReader<'_, '_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.properties.read_embedded(buffer)
    }
}

impl<R: Read> EmbeddedInput for TnefPropertyReader<'_, R> {
    fn read_embedded(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.run(|properties| {
            let count = buffer
                .len()
                .min((properties.data_end - properties.reader.position) as usize);
            properties
                .reader
                .read_raw(&mut buffer[..count], &mut properties.logger)
        })
        .map_err(|error| match error {
            Error::Io(error) => error,
            error => io::Error::other(error),
        })
    }
}

fn decode_unicode(bytes: &[u8]) -> String {
    // Unicode strings are usually NUL-terminated. Discard an odd final byte,
    // then strip complete UTF-16 terminators, not individual zero bytes.
    let mut length = bytes.len() & !1;
    while length >= 2 && bytes[length - 2..length] == [0, 0] {
        length -= 2;
    }
    char::decode_utf16(
        bytes[..length]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]])),
    )
    .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
    .collect()
}

fn fixed_value(kind: Type, bytes: &[u8; 20]) -> TnefValue {
    let word = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let long = i64::from_le_bytes(bytes[..8].try_into().unwrap());
    match kind {
        Type::NULL => TnefValue::Null,
        Type::I2 => TnefValue::I16(word as i16),
        Type::LONG => TnefValue::I32(word as i32),
        Type::ERROR => TnefValue::Error(word),
        Type::BOOLEAN => TnefValue::Bool(word & 0xffff != 0),
        Type::R4 => TnefValue::F32(f32::from_bits(word)),
        Type::DOUBLE => TnefValue::F64(f64::from_bits(long as u64)),
        Type::CURRENCY => TnefValue::Currency(long),
        Type::APP_TIME => TnefValue::AppTime(f64::from_bits(long as u64)),
        Type::SYS_TIME => TnefValue::SysTime(long),
        Type::I8 => TnefValue::I64(long),
        Type::CLASS_ID => TnefValue::Guid(bytes[..16].try_into().unwrap()),
        _ => unreachable!("fixed-width type checked before decoding"),
    }
}

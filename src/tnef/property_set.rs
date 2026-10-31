// Adapted from MimeKit's TnefProperty.cs and TnefPropertySet.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, ops::Index};

use super::{
    TnefDateTime, TnefNameId, TnefPropertyInfo, TnefPropertyTag, TnefPropertyType, TnefValue,
    TnefValueConversionError as ConversionError,
};
use crate::charsets::Charset;

/// An owned, read-only snapshot of a property and its native values.
///
/// Values survive reader advancement and destruction and can be read in any
/// order. Binary/object data is held in memory, including object interface
/// GUIDs; use the cursor's bounded stream for large values instead.
///
/// Dates and currency retain the wire representations described by
/// [`TnefValue`]. Typed accessors convert explicitly without changing raw values.
/// Binary-to-text conversion uses the charset captured when the snapshot was read.
#[derive(Clone, Debug, PartialEq)]
pub struct TnefProperty {
    info: TnefPropertyInfo,
    values: Vec<TnefValue>,
    charset: Charset,
}

impl TnefProperty {
    pub(super) fn new(info: TnefPropertyInfo, values: Vec<TnefValue>, charset: Charset) -> Self {
        Self {
            info,
            values,
            charset,
        }
    }

    /// Captured charset used to interpret Binary values as text.
    pub fn charset(&self) -> Charset {
        self.charset
    }

    /// Original tag, name, and stream offset.
    pub fn info(&self) -> &TnefPropertyInfo {
        &self.info
    }

    /// Full wire tag. Named-property identifiers are local to their stream;
    /// use the GUID/name descriptor to identify them across streams.
    pub fn tag(&self) -> TnefPropertyTag {
        self.info.tag
    }

    /// Value type without the multivalue flag.
    pub fn property_type(&self) -> TnefPropertyType {
        self.info.tag.value_type()
    }

    /// Whether the wire tag has the multivalue flag.
    pub fn is_multi_valued(&self) -> bool {
        self.info.tag.is_multi_valued()
    }

    /// Property-set GUID and numeric or case-sensitive text name, when present.
    pub fn name(&self) -> Option<&TnefNameId> {
        self.info.name.as_ref()
    }

    /// Number of values actually read, which can be smaller than declared.
    pub fn value_count(&self) -> usize {
        self.values.len()
    }

    /// Native values in wire order, including for a single-valued property.
    pub fn values(&self) -> &[TnefValue] {
        &self.values
    }

    /// The single value, or `None` for an empty or multivalued property.
    /// A Null value is `Some(TnefValue::Null)`, distinct from no value.
    pub fn value(&self) -> Option<&TnefValue> {
        if self.is_multi_valued() {
            None
        } else {
            self.values.first()
        }
    }

    fn scalar(&self) -> Result<&TnefValue, ConversionError> {
        if self.is_multi_valued() {
            return Err(ConversionError::MultiValued);
        }
        self.values.first().ok_or(ConversionError::NoValue)
    }

    /// Converts a single numeric/boolean value; floating-point values are rejected.
    ///
    /// # Errors
    ///
    /// Returns explicit missing/multivalue or [`TnefValue::to_bool`] errors.
    pub fn to_bool(&self) -> Result<bool, ConversionError> {
        self.scalar()?.to_bool()
    }

    /// Converts through i64 and retains the low 16 bits, as MimeKit does.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or [`TnefValue::to_i16`] errors.
    pub fn to_i16(&self) -> Result<i16, ConversionError> {
        self.scalar()?.to_i16()
    }

    /// Converts through i64 and retains the low 32 bits, as MimeKit does.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or [`TnefValue::to_i32`] errors.
    pub fn to_i32(&self) -> Result<i32, ConversionError> {
        self.scalar()?.to_i32()
    }

    /// Converts a single numeric/boolean value, truncating fractions toward zero.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or [`TnefValue::to_i64`] errors.
    pub fn to_i64(&self) -> Result<i64, ConversionError> {
        self.scalar()?.to_i64()
    }

    /// Converts a single numeric/boolean value to f32.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or [`TnefValue::to_f32`] errors.
    pub fn to_f32(&self) -> Result<f32, ConversionError> {
        self.scalar()?.to_f32()
    }

    /// Converts a single numeric/boolean value to f64.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or [`TnefValue::to_f64`] errors.
    pub fn to_f64(&self) -> Result<f64, ConversionError> {
        self.scalar()?.to_f64()
    }

    /// Converts a single AppTime/SysTime without losing its time kind or precision.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or wrapped [`TnefValue::to_datetime`] errors.
    pub fn to_datetime(&self) -> Result<TnefDateTime, ConversionError> {
        self.scalar()?.to_datetime().map_err(|error| match error {
            super::TnefDateTimeError::NotDate => ConversionError::InvalidType,
            error => ConversionError::InvalidDate(error),
        })
    }

    /// Returns a single native GUID in Windows wire order.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or [`TnefValue::to_guid`] errors.
    pub fn to_guid(&self) -> Result<[u8; 16], ConversionError> {
        self.scalar()?.to_guid()
    }

    /// Borrows single Binary/Object/GUID bytes, retaining any object IID.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or [`TnefValue::as_bytes`] errors.
    pub fn as_bytes(&self) -> Result<&[u8], ConversionError> {
        self.scalar()?.as_bytes()
    }

    /// Borrows a string, or decodes Binary using the snapshot's original charset.
    /// Binary decoding strips trailing NUL bytes and uses replacement decoding.
    /// Object values are not interpreted as text.
    /// Decoded strings are not cached; any output allocations belong to the
    /// caller and do not consume the former reader's wire-byte budget.
    ///
    /// # Errors
    ///
    /// Returns missing/multivalue or `InvalidType` errors.
    pub fn to_text(&self) -> Result<Cow<'_, str>, ConversionError> {
        match self.scalar()? {
            TnefValue::Text(value) => Ok(Cow::Borrowed(value)),
            TnefValue::Binary(value) => Ok(super::reader::decode_string(self.charset, value)),
            _ => Err(ConversionError::InvalidType),
        }
    }

    /// Returns the metadata and values without cloning.
    /// The decoding charset is available separately through [`charset`](Self::charset).
    pub fn into_parts(self) -> (TnefPropertyInfo, Vec<TnefValue>) {
        (self.info, self.values)
    }
}

/// An owned, ordered, read-only set of MAPI properties or one recipient row.
///
/// Duplicates are preserved; identifier and named lookups return the first
/// match even if it has no value. Indexing uses ordinary slice bounds checks.
/// Collections may be partial after logged malformed framing or truncation;
/// obtaining a set does not verify the containing attribute's checksum.
///
/// ```
/// use mimecraft::tnef::{TnefProperty, TnefPropertyTag, TnefReader, TnefValue};
/// # let input = b"\x78\x9f\x3e\x22\0\0\x01\x03\x90\x06\0\x0c\0\0\0\x01\0\0\0\x03\0\x17\0\x07\0\0\0\x22\0";
/// let mut reader = TnefReader::new(input.as_slice());
/// assert!(reader.read_next()?);
/// let set = reader.open_properties()?.read_property_set()?;
/// // Verify the containing attribute's checksum before using the snapshot.
/// assert!(!reader.read_next()?);
/// assert_eq!(set.get_by_id(0x0017).and_then(TnefProperty::value),
///            Some(&TnefValue::I32(7)));
/// assert_eq!(set.get_i32(TnefPropertyTag::IMPORTANCE)?, Some(7));
/// assert_eq!(set.get_text(TnefPropertyTag::SUBJECT_W)?, None);
/// // A present property with an incompatible type is an error, not absence.
/// assert!(set.get_text(TnefPropertyTag::IMPORTANCE).is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TnefPropertySet {
    properties: Vec<TnefProperty>,
}

impl TnefPropertySet {
    /// Creates an empty set without allocating.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of properties, including duplicates.
    pub fn len(&self) -> usize {
        self.properties.len()
    }

    /// Whether the set contains no properties.
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }

    /// Borrows the properties in their original order.
    pub fn as_slice(&self) -> &[TnefProperty] {
        &self.properties
    }

    /// Iterates over properties in order, including duplicates.
    pub fn iter(&self) -> std::slice::Iter<'_, TnefProperty> {
        self.properties.iter()
    }

    /// Returns the stored vector without cloning.
    pub fn into_vec(self) -> Vec<TnefProperty> {
        self.properties
    }

    /// Gets the first property with the tag's identifier, ignoring its type.
    ///
    /// Thus a Unicode subject tag also finds a String8 subject. Named-property
    /// identifiers are meaningful only within the original stream; use
    /// [`get_named`](Self::get_named) to look them up by GUID/name.
    pub fn get(&self, tag: TnefPropertyTag) -> Option<&TnefProperty> {
        self.get_by_id(tag.id())
    }

    /// Gets the first property with the given identifier, even without a value.
    pub fn get_by_id(&self, id: u16) -> Option<&TnefProperty> {
        self.properties
            .iter()
            .find(|property| property.tag().id() == id)
    }

    /// Gets the first property with the exact GUID/name descriptor.
    /// Text names are case-sensitive; numeric and textual names are distinct.
    pub fn get_named(&self, name: &TnefNameId) -> Option<&TnefProperty> {
        self.properties
            .iter()
            .find(|property| property.name() == Some(name))
    }
}

macro_rules! getters {
    ($($name:ident, $method:ident, $result:ty, $doc:literal;)+) => {
        impl TnefPropertySet {
            $(
                #[doc = $doc]
                ///
                /// Returns `None` only when the identifier is absent. The first
                /// matching property wins regardless of type or convertibility.
                ///
                /// # Errors
                ///
                /// Returns the property's explicit scalar conversion error.
                pub fn $name(&self, tag: TnefPropertyTag) -> Result<Option<$result>, ConversionError> {
                    self.get(tag).map(TnefProperty::$method).transpose()
                }
            )+
        }
    };
}

getters! {
    get_bool, to_bool, bool, "Gets the first matching property as a boolean.";
    get_i16, to_i16, i16, "Gets the first matching property as a low-16-bit integer.";
    get_i32, to_i32, i32, "Gets the first matching property as a low-32-bit integer.";
    get_i64, to_i64, i64, "Gets the first matching property as an integer.";
    get_f32, to_f32, f32, "Gets the first matching property as f32.";
    get_f64, to_f64, f64, "Gets the first matching property as f64.";
    get_datetime, to_datetime, TnefDateTime, "Gets the first matching property as a checked date.";
    get_guid, to_guid, [u8; 16], "Gets the first matching property as a native GUID.";
    get_bytes, as_bytes, &[u8], "Borrows the first matching property's Binary/Object/GUID bytes.";
    get_text, to_text, Cow<'_, str>, "Gets text using the first matching property's captured charset.";
}

impl From<Vec<TnefProperty>> for TnefPropertySet {
    fn from(properties: Vec<TnefProperty>) -> Self {
        Self { properties }
    }
}

impl FromIterator<TnefProperty> for TnefPropertySet {
    fn from_iter<T: IntoIterator<Item = TnefProperty>>(iter: T) -> Self {
        Self {
            properties: iter.into_iter().collect(),
        }
    }
}

impl Index<usize> for TnefPropertySet {
    type Output = TnefProperty;
    fn index(&self, index: usize) -> &Self::Output {
        &self.properties[index]
    }
}

impl<'a> IntoIterator for &'a TnefPropertySet {
    type Item = &'a TnefProperty;
    type IntoIter = std::slice::Iter<'a, TnefProperty>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for TnefPropertySet {
    type Item = TnefProperty;
    type IntoIter = std::vec::IntoIter<TnefProperty>;
    fn into_iter(self) -> Self::IntoIter {
        self.properties.into_iter()
    }
}

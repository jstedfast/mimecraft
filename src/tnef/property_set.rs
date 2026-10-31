// Adapted from MimeKit's TnefProperty.cs and TnefPropertySet.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::ops::Index;

use super::{TnefNameId, TnefPropertyInfo, TnefPropertyTag, TnefPropertyType, TnefValue};

/// An owned, read-only snapshot of a property and its native values.
///
/// Values survive reader advancement and destruction and can be read in any
/// order. Binary/object data is held in memory, including object interface
/// GUIDs; use the cursor's bounded stream for large values instead.
///
/// Dates and currency retain the wire representations described by
/// [`TnefValue`]. There are no implicit numeric or binary-to-string coercions.
#[derive(Clone, Debug, PartialEq)]
pub struct TnefProperty {
    info: TnefPropertyInfo,
    values: Vec<TnefValue>,
}

impl TnefProperty {
    pub(super) fn new(info: TnefPropertyInfo, values: Vec<TnefValue>) -> Self {
        Self { info, values }
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

    /// Returns the metadata and values without cloning.
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
/// use mimecraft::tnef::{TnefProperty, TnefReader, TnefValue};
/// # let input = b"\x78\x9f\x3e\x22\0\0\x01\x03\x90\x06\0\x0c\0\0\0\x01\0\0\0\x03\0\x17\0\x07\0\0\0\x22\0";
/// let mut reader = TnefReader::new(input.as_slice());
/// assert!(reader.read_next()?);
/// let set = reader.open_properties()?.read_property_set()?;
/// // Verify the containing attribute's checksum before using the snapshot.
/// assert!(!reader.read_next()?);
/// assert_eq!(set.get_by_id(0x0017).and_then(TnefProperty::value),
///            Some(&TnefValue::I32(7)));
/// # Ok::<(), mimecraft::tnef::TnefReadError>(())
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

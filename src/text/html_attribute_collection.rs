// Adapted from MimeKit's Text/HtmlAttributeCollection.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::ops::Index;

use super::{HtmlAttribute, HtmlAttributeId};

/// An ordered, read-only collection of HTML attributes.
///
/// Duplicate names and their original case are preserved. Name and identifier
/// lookups return the first match, including valueless attributes. Matching by
/// name uses ASCII case-insensitive comparison; non-ASCII characters compare
/// exactly, rather than using .NET's Unicode ordinal case folding.
///
/// Construct with [`From<Vec<HtmlAttribute>>`] or collect an iterator. Use
/// [`Self::as_slice`] for checked positional access and [`Self::into_vec`] to
/// recover ownership. Indexing follows ordinary slice bounds checking.
///
/// ```
/// use mimecraft::text::{HtmlAttribute, HtmlAttributeCollection, HtmlAttributeId};
///
/// let attributes: HtmlAttributeCollection = vec![
///     HtmlAttribute::new("ALT", None)?,
///     HtmlAttribute::from_id(HtmlAttributeId::Alt, Some("second".into()))?,
/// ].into();
/// assert_eq!(attributes.len(), 2);
/// assert_eq!(attributes.index_of("alt"), Some(0));
/// assert_eq!(attributes.get_by_id(HtmlAttributeId::Alt).unwrap().value(), None);
/// # Ok::<(), mimecraft::text::HtmlAttributeError>(())
/// ```
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct HtmlAttributeCollection {
    attributes: Vec<HtmlAttribute>,
}

impl HtmlAttributeCollection {
    /// Creates an empty collection without allocating.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of attributes, including duplicates.
    pub fn len(&self) -> usize {
        self.attributes.len()
    }

    /// Returns whether there are no attributes.
    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    /// Borrows the stored attributes in their original order.
    pub fn as_slice(&self) -> &[HtmlAttribute] {
        &self.attributes
    }

    /// Iterates over stored attributes in order, including duplicates.
    pub fn iter(&self) -> std::slice::Iter<'_, HtmlAttribute> {
        self.attributes.iter()
    }

    /// Returns the stored vector without cloning attributes.
    pub fn into_vec(self) -> Vec<HtmlAttribute> {
        self.attributes
    }

    /// Gets the first attribute with the given ASCII case-insensitive name.
    ///
    /// Returns an attribute even when it has no value. Lookup does not validate,
    /// trim, or decode the supplied name.
    pub fn get(&self, name: &str) -> Option<&HtmlAttribute> {
        self.index_of(name).map(|index| &self.attributes[index])
    }

    /// Gets the first attribute with the given identifier, even without a value.
    ///
    /// [`HtmlAttributeId::Unknown`] matches the first unrecognized name.
    pub fn get_by_id(&self, id: HtmlAttributeId) -> Option<&HtmlAttribute> {
        self.index_of_id(id).map(|index| &self.attributes[index])
    }

    /// Tests whether the given ASCII case-insensitive name is present.
    pub fn contains(&self, name: &str) -> bool {
        self.index_of(name).is_some()
    }

    /// Tests whether the identifier is present, including unrecognized names.
    pub fn contains_id(&self, id: HtmlAttributeId) -> bool {
        self.index_of_id(id).is_some()
    }

    /// Gets the first index matching the given ASCII case-insensitive name.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.attributes
            .iter()
            .position(|attribute| attribute.name().eq_ignore_ascii_case(name))
    }

    /// Gets the first index matching an identifier.
    ///
    /// [`HtmlAttributeId::Unknown`] matches the first unrecognized name.
    pub fn index_of_id(&self, id: HtmlAttributeId) -> Option<usize> {
        self.attributes
            .iter()
            .position(|attribute| attribute.id() == id)
    }
}

impl From<Vec<HtmlAttribute>> for HtmlAttributeCollection {
    fn from(attributes: Vec<HtmlAttribute>) -> Self {
        Self { attributes }
    }
}

impl FromIterator<HtmlAttribute> for HtmlAttributeCollection {
    fn from_iter<T: IntoIterator<Item = HtmlAttribute>>(iter: T) -> Self {
        Self {
            attributes: iter.into_iter().collect(),
        }
    }
}

impl Index<usize> for HtmlAttributeCollection {
    type Output = HtmlAttribute;

    fn index(&self, index: usize) -> &Self::Output {
        &self.attributes[index]
    }
}

impl<'a> IntoIterator for &'a HtmlAttributeCollection {
    type Item = &'a HtmlAttribute;
    type IntoIter = std::slice::Iter<'a, HtmlAttribute>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for HtmlAttributeCollection {
    type Item = HtmlAttribute;
    type IntoIter = std::vec::IntoIter<HtmlAttribute>;

    fn into_iter(self) -> Self::IntoIter {
        self.attributes.into_iter()
    }
}

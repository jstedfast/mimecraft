// Adapted from MimeKit's HeaderList.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    collections::HashMap,
    error::Error,
    fmt,
    io::{self, BufRead, Write},
    iter::FusedIterator,
    num::NonZeroUsize,
};

use crate::{
    Header, HeaderParser,
    encodings::NewLineFormat,
    filters::{Dos2UnixFilter, MimeFilter, Unix2DosFilter},
    io::FilteredWriter,
};

/// An ordered collection of raw headers, retaining duplicate fields.
///
/// Field-name lookup ignores ASCII case and returns the first occurrence;
/// [`get_all`](Self::get_all) visits every occurrence in order. Records retain
/// their original bytes, including folded values, spelling, and malformed
/// syntax. Collection edits neither decode nor reformat individual headers.
/// Moving an unterminated record does not add a separating newline.
///
/// Construct from a [`Vec<Header>`] produced by [`crate::HeaderParser`], collect
/// an iterator of headers, or extend a list with successive parser batches.
/// The parser's body separator and consumed-input count remain separate; this
/// collection does not parse bodies or track a header/body boundary.
/// [`load`](Self::load) reads a block from a buffered source without consuming
/// body bytes; use [`HeaderParser::read_from`] to also retain separator metadata.
///
/// Malformed records participate in positional operations and iteration but
/// never match a field-name query. Empty names accepted by the raw parser do
/// match `""`. An unknown extension field is looked up by its actual name, not
/// by the shared [`crate::HeaderId::Unknown`] identifier.
///
/// A case-insensitive dictionary indexes each field's first occurrence.
/// [`get`](Self::get), [`contains`](Self::contains), and
/// [`index_of`](Self::index_of) take expected O(1) time in the number of headers,
/// plus O(m) to normalize/hash a query of length m. All-occurrence and
/// last-occurrence queries still scan the ordered list.
/// Rust ownership replaces shared mutable headers and change events; mutation
/// methods maintain the index. Typed values create checked records through
/// [`crate::AddressList::to_header`], [`crate::ContentType::to_header`], and
/// [`crate::ContentDisposition::to_header`]; construct those before calling
/// [`replace_all`](Self::replace_all) to leave the list intact on encoding errors.
/// Serialization can preserve raw bytes or normalize line endings, but does
/// not re-encode values or insert a header/body separator.
///
/// ```
/// use mimecraft::{HeaderList, HeaderParser};
///
/// let input = b"Received: first\r\nReceived: second\r\n\r\nbody";
/// let mut parser = HeaderParser::new();
/// let mut records = Vec::new();
/// let consumed = parser.parse(input, &mut records)?;
/// let mut headers = HeaderList::from(records);
/// assert_eq!(&input[consumed..], b"body");
/// assert_eq!(headers.get_all("received").count(), 2);
/// assert_eq!(headers.remove("RECEIVED").unwrap().raw_value(), b" first\r\n");
/// assert_eq!(headers.get("Received").unwrap().raw_value(), b" second\r\n");
/// # Ok::<(), mimecraft::HeaderTooLong>(())
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeaderList {
    headers: Vec<Header>,
    // Like MimeKit's lookup table, this references the first header of each
    // field. Lowercase keys do not alter the original spelling in the records.
    first_by_name: HashMap<String, usize>,
}

impl HeaderList {
    /// Creates an empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty collection with room for at least `capacity` records.
    ///
    /// This reserves list and dictionary storage, not individual header buffers
    /// or dictionary keys.
    pub fn with_capacity(capacity: usize) -> Self {
        // When a parser knows the header count, pre-sizing avoids repeated
        // list growth and dictionary rehashing, as in MimeKit's constructor.
        Self {
            headers: Vec::with_capacity(capacity),
            first_by_name: HashMap::with_capacity(capacity),
        }
    }

    /// Loads raw headers through the first body separator or EOF.
    ///
    /// Retains duplicates, malformed records (including the first), and exact
    /// header bytes. Uses the default 16 KiB per-record limit. Body bytes remain
    /// available through the same buffered reader; do not discard that buffer.
    /// Empty input yields an empty list. No seeking or closing is performed.
    ///
    /// The separator is consumed but not stored in this collection. To retain
    /// it, use [`HeaderParser::read_from`] and convert its output into a list.
    /// Unlike MimeKit's higher-level parser, this accepts an invalid first field.
    ///
    /// # Errors
    ///
    /// Propagates read failures and returns [`io::ErrorKind::InvalidData`] with
    /// [`crate::HeaderTooLong`] for an oversized record. Interrupted reads retry.
    /// On error, partial headers are discarded and source position is not
    /// restored; retrying this method does not resume the failed block.
    /// Use a persistent [`HeaderParser`] for retryable I/O and partial output.
    ///
    /// ```
    /// use mimecraft::HeaderList;
    ///
    /// let mut input = &b"Subject: hello\r\n\r\nbody"[..];
    /// let headers = HeaderList::load(&mut input)?;
    /// assert_eq!(headers.get("subject").unwrap().decode_value().text, "hello");
    /// assert_eq!(input, b"body");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn load<R: BufRead + ?Sized>(reader: &mut R) -> io::Result<Self> {
        let mut records = Vec::new();
        HeaderParser::new().read_from(reader, &mut records)?;
        Ok(Self::from(records))
    }

    /// Loads headers with a positive maximum byte length per record.
    ///
    /// Otherwise has the same behavior and errors as [`load`](Self::load).
    /// Neither method limits the total byte size or number of records; callers
    /// must impose those limits separately when required.
    pub fn load_with_max_header_length<R: BufRead + ?Sized>(
        reader: &mut R,
        limit: NonZeroUsize,
    ) -> io::Result<Self> {
        let mut records = Vec::new();
        HeaderParser::with_max_header_length(limit).read_from(reader, &mut records)?;
        Ok(Self::from(records))
    }

    /// Writes stored records verbatim, without flushing the destination.
    ///
    /// Does not add missing record newlines or a header/body separator. In
    /// particular, moving an unterminated record before another concatenates
    /// their bytes without repairing the boundary. No validation is performed.
    ///
    /// # Errors
    ///
    /// Propagates write errors, including [`io::ErrorKind::WriteZero`], after
    /// any partial output. Interrupted writes retry. Restarting this method
    /// repeats output from the beginning; it does not resume a failed write.
    pub fn write_raw_to<W: Write + ?Sized>(&self, output: &mut W) -> io::Result<()> {
        for header in &self.headers {
            output.write_all(header.raw())?;
        }
        Ok(())
    }

    /// Writes headers with CRLF line endings and flushes the destination.
    ///
    /// See [`write_to_with_new_line_format`](Self::write_to_with_new_line_format)
    /// for normalization rules and errors. Does not add a body separator.
    pub fn write_to<W: Write + ?Sized>(&self, output: &mut W) -> io::Result<()> {
        self.write_to_with_new_line_format(output, NewLineFormat::Dos)
    }

    /// Writes headers with CRLF or LF line endings, then flushes the destination.
    ///
    /// Adapts MimeKit's raw header-list serialization: preserves field spelling,
    /// folding whitespace, value bytes, and malformed records while converting
    /// newlines. Bare CR bytes are retained. An invalid carriage-return-only
    /// first line is written verbatim so repeated conversion cannot turn it
    /// into a blank line and change the message structure.
    ///
    /// No values are decoded, re-encoded, or refolded; missing final newlines
    /// are not added. Unlike MimeKit's separator-tracking list, this collection
    /// never appends the blank header/body separator. Write it separately when
    /// needed, after ensuring the last record has a terminating newline.
    /// The list is unchanged, and the borrowed destination is not closed.
    ///
    /// # Errors
    ///
    /// `Mixed` returns [`io::ErrorKind::InvalidInput`] before any output or flush.
    /// Write and flush errors propagate, possibly after partial or complete
    /// output. Interrupted writes retry; zero writes fail with `WriteZero`.
    /// Retrying the method starts over, not at the failed position.
    pub fn write_to_with_new_line_format<W: Write + ?Sized>(
        &self,
        output: &mut W,
        format: NewLineFormat,
    ) -> io::Result<()> {
        self.write_normalized(output, format, false)
    }

    pub(crate) fn write_terminated<W: Write + ?Sized>(
        &self,
        output: &mut W,
        format: NewLineFormat,
    ) -> io::Result<()> {
        self.write_normalized(output, format, true)
    }

    fn write_normalized<W: Write + ?Sized>(
        &self,
        output: &mut W,
        format: NewLineFormat,
        terminate_records: bool,
    ) -> io::Result<()> {
        match format {
            NewLineFormat::Dos => {
                self.write_filtered(output, Unix2DosFilter::new(), terminate_records)
            }
            NewLineFormat::Unix => {
                self.write_filtered(output, Dos2UnixFilter::new(), terminate_records)
            }
            NewLineFormat::Mixed => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "header new-line format must be Dos or Unix",
            )),
        }
    }

    fn write_filtered<W: Write + ?Sized, F: MimeFilter>(
        &self,
        output: &mut W,
        filter: F,
        terminate_records: bool,
    ) -> io::Result<()> {
        let mut filtered = FilteredWriter::new(output, filter);
        for header in &self.headers {
            let raw = header.raw();
            let mut prefix = 0;
            if header.is_invalid() {
                let carriage_returns = raw.iter().take_while(|&&byte| byte == b'\r').count();
                if carriage_returns > 0 && raw.get(carriage_returns) == Some(&b'\n') {
                    prefix = carriage_returns + 1;
                }
            }
            if prefix > 0 {
                // A CR-only invalid line is not a body separator. Dos2Unix
                // would strip a CR on each round trip until it became one.
                // Drain and finalize before bypassing the filter; continuation
                // lines still go through normal newline conversion.
                filtered.finish()?;
                let (output, filter, _) = filtered.into_parts();
                output.write_all(&raw[..prefix])?;
                filtered = FilteredWriter::new(output, filter);
            }
            filtered.write_all(&raw[prefix..])?;
            if terminate_records && !raw.ends_with(b"\n") {
                // Entity output needs distinct records and a separate blank
                // separator, even when a record came from truncated input.
                // Passing LF through the filter also completes a trailing CR.
                filtered.write_all(b"\n")?;
            }
        }
        filtered.finish()
    }

    /// Returns the number of records, including duplicates and malformed entries.
    pub fn len(&self) -> usize {
        self.headers.len()
    }

    /// Returns whether the collection contains no records.
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }

    /// Returns all records in their current order.
    pub fn as_slice(&self) -> &[Header] {
        &self.headers
    }

    /// Iterates over records in list order.
    pub fn iter(&self) -> std::slice::Iter<'_, Header> {
        self.headers.iter()
    }

    /// Consumes the collection and returns its vector without copying records.
    pub fn into_vec(self) -> Vec<Header> {
        self.headers
    }

    /// Returns the first matching field, ignoring ASCII case.
    pub fn get(&self, field: &str) -> Option<&Header> {
        self.index_of(field).map(|index| &self.headers[index])
    }

    /// Iterates over all matching fields in order, without allocating.
    ///
    /// The iterator borrows both the collection and the query string. It also
    /// supports reverse iteration. Malformed records never match.
    pub fn get_all<'a>(
        &'a self,
        field: &'a str,
    ) -> impl DoubleEndedIterator<Item = &'a Header> + FusedIterator + 'a {
        self.headers
            .iter()
            .filter(move |header| header.matches_field(field))
    }

    /// Returns whether a matching field is present, ignoring ASCII case.
    pub fn contains(&self, field: &str) -> bool {
        self.index_of(field).is_some()
    }

    /// Returns the first matching position, ignoring ASCII case.
    pub fn index_of(&self, field: &str) -> Option<usize> {
        self.first_by_name.get(field_key(field).as_ref()).copied()
    }

    /// Returns the last matching position, ignoring ASCII case.
    pub fn last_index_of(&self, field: &str) -> Option<usize> {
        self.headers
            .iter()
            .rposition(|header| header.matches_field(field))
    }

    /// Appends a record, allowing duplicate fields and malformed records.
    pub fn add(&mut self, header: Header) {
        let index = self.len();
        self.headers.push(header);
        self.index_header(index);
    }

    /// Inserts before `index`, or appends when `index == len()`.
    ///
    /// Duplicate fields and malformed records are allowed. Inserting a field
    /// before its previous first occurrence changes subsequent field lookup.
    ///
    /// # Errors
    ///
    /// Returns an index error if `index > len()`, leaving the collection unchanged.
    pub fn insert(&mut self, index: usize, header: Header) -> Result<(), HeaderListEditError> {
        if index > self.len() {
            return Err(HeaderListEditError::IndexOutOfBounds {
                index,
                len: self.len(),
            });
        }
        if index == self.len() {
            self.add(header);
            return Ok(());
        }
        // Insertion moves every indexed first occurrence at or after this slot.
        // The inserted field may then become the new first occurrence itself.
        for first in self.first_by_name.values_mut() {
            if *first >= index {
                *first += 1;
            }
        }
        self.headers.insert(index, header);
        self.index_header(index);
        Ok(())
    }

    /// Replaces one record by position and returns the previous record.
    ///
    /// The replacement may have a different field name or be malformed.
    /// Other records, including duplicates, are left untouched. To collapse
    /// duplicates by field name instead, use [`replace_all`](Self::replace_all).
    ///
    /// # Errors
    ///
    /// Returns an index error if `index >= len()`, leaving the collection unchanged.
    pub fn replace(&mut self, index: usize, header: Header) -> Result<Header, HeaderListEditError> {
        let len = self.len();
        let previous = self
            .headers
            .get_mut(index)
            .ok_or(HeaderListEditError::IndexOutOfBounds { index, len })?;
        let previous = std::mem::replace(previous, header);
        if !previous
            .field()
            .is_some_and(|field| self.headers[index].matches_field(field))
        {
            if let Some(field) = previous.field() {
                self.repair_first(field, index);
            }
            self.index_header(index);
        }
        Ok(previous)
    }

    /// Replaces all occurrences of the supplied header's field with that record.
    ///
    /// Matches MimeKit's `Replace(Header)`: replaces at the first occurrence's
    /// position and discards later duplicates without reordering other fields.
    /// If the field is absent, appends the record. Adopts all replacement bytes,
    /// including field spelling, rather than modifying the old record's value.
    ///
    /// Returns the previous first record, or `None` if appended. Later matching
    /// records are dropped, not included in the return value.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderListEditError::InvalidHeader`] if the replacement has no
    /// parsed field name, leaving the collection unchanged. Empty names accepted
    /// by the raw parser are allowed; this is not authoring validation.
    pub fn replace_all(&mut self, header: Header) -> Result<Option<Header>, HeaderListEditError> {
        let field = header.field().ok_or(HeaderListEditError::InvalidHeader)?;
        let Some(first) = self.index_of(field) else {
            self.add(header);
            return Ok(None);
        };
        // Keep the first matching slot: removing all matches and appending
        // would move the replacement past unrelated headers. Remove only later
        // matches, retaining their neighbors' relative order.
        let mut index = 0;
        self.headers.retain(|candidate| {
            let keep = index <= first || !candidate.matches_field(field);
            index += 1;
            keep
        });
        let previous = std::mem::replace(&mut self.headers[first], header);
        self.rebuild_index();
        Ok(Some(previous))
    }

    /// Removes the first matching field, ignoring ASCII case, and returns it.
    ///
    /// Later duplicates remain, and the next one becomes the first lookup result.
    pub fn remove(&mut self, field: &str) -> Option<Header> {
        self.index_of(field).map(|index| self.remove_index(index))
    }

    /// Removes every matching field and returns the number removed.
    ///
    /// Matching ignores ASCII case; all other records retain their order and bytes.
    pub fn remove_all(&mut self, field: &str) -> usize {
        if !self.contains(field) {
            return 0;
        }
        let previous = self.len();
        self.headers.retain(|header| !header.matches_field(field));
        self.rebuild_index();
        previous - self.len()
    }

    /// Removes one record by position and returns it, preserving remaining order.
    ///
    /// # Errors
    ///
    /// Returns an index error if `index >= len()`, leaving the collection unchanged.
    pub fn remove_at(&mut self, index: usize) -> Result<Header, HeaderListEditError> {
        if index >= self.len() {
            return Err(HeaderListEditError::IndexOutOfBounds {
                index,
                len: self.len(),
            });
        }
        Ok(self.remove_index(index))
    }

    /// Removes all records and dictionary keys, retaining both capacities.
    pub fn clear(&mut self) {
        self.headers.clear();
        self.first_by_name.clear();
    }

    fn index_header(&mut self, index: usize) {
        if let Some(field) = self.headers[index].field() {
            // Entry avoids probing twice, like MimeKit's TryAdd, while keeping
            // the earlier duplicate when a record is appended or inserted.
            self.first_by_name
                .entry(field.to_ascii_lowercase())
                .and_modify(|first| *first = (*first).min(index))
                .or_insert(index);
        }
    }

    fn repair_first(&mut self, field: &str, index: usize) {
        let key = field_key(field);
        if let Some(first) = self.first_by_name.get_mut(key.as_ref())
            && *first == index
        {
            // If the first occurrence was removed or renamed, find the next
            // matching header and promote it into the lookup table.
            if let Some(next) = self.headers[index..]
                .iter()
                .position(|header| header.matches_field(field))
            {
                *first = index + next;
            } else {
                self.first_by_name.remove(key.as_ref());
            }
        }
    }

    fn remove_index(&mut self, index: usize) -> Header {
        let header = self.headers.remove(index);
        for first in self.first_by_name.values_mut() {
            if *first > index {
                *first -= 1;
            }
        }
        if let Some(field) = header.field() {
            self.repair_first(field, index);
        }
        header
    }

    fn rebuild_index(&mut self) {
        // Bulk compaction can shift many fields by different amounts. Rebuild
        // once from the final order rather than repairing each removal in turn.
        self.first_by_name.clear();
        for index in 0..self.len() {
            self.index_header(index);
        }
    }
}

fn field_key(field: &str) -> Cow<'_, str> {
    if field.bytes().any(|byte| byte.is_ascii_uppercase()) {
        Cow::Owned(field.to_ascii_lowercase())
    } else {
        Cow::Borrowed(field)
    }
}

impl From<Vec<Header>> for HeaderList {
    fn from(headers: Vec<Header>) -> Self {
        let mut list = Self {
            first_by_name: HashMap::with_capacity(headers.len()),
            headers,
        };
        list.rebuild_index();
        list
    }
}

impl FromIterator<Header> for HeaderList {
    fn from_iter<T: IntoIterator<Item = Header>>(iter: T) -> Self {
        Self::from(iter.into_iter().collect::<Vec<_>>())
    }
}

impl Extend<Header> for HeaderList {
    fn extend<T: IntoIterator<Item = Header>>(&mut self, iter: T) {
        let iter = iter.into_iter();
        let additional = iter.size_hint().0;
        self.headers.reserve(additional);
        self.first_by_name.reserve(additional);
        for header in iter {
            self.add(header);
        }
    }
}

impl<'a> IntoIterator for &'a HeaderList {
    type Item = &'a Header;
    type IntoIter = std::slice::Iter<'a, Header>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for HeaderList {
    type Item = Header;
    type IntoIter = std::vec::IntoIter<Header>;

    fn into_iter(self) -> Self::IntoIter {
        self.headers.into_iter()
    }
}

/// An unsuccessful ordered header-list edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderListEditError {
    /// The index is outside the range accepted by the operation.
    IndexOutOfBounds {
        /// The index supplied by the caller.
        index: usize,
        /// The current record count.
        len: usize,
    },
    /// A field-based replacement requires a parsed field name.
    ///
    /// Malformed records can still be added, inserted, or replaced by position.
    InvalidHeader,
}

impl fmt::Display for HeaderListEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexOutOfBounds { index, len } => {
                write!(f, "header index {index} is out of bounds for length {len}")
            }
            Self::InvalidHeader => f.write_str("replacement header has no parsed field name"),
        }
    }
}

impl Error for HeaderListEditError {}

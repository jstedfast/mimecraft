// Adapted from MimeKit's TnefReader.cs, TnefReaderStream.cs, and TnefChecksum.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    cell::RefCell,
    io::{self, Read},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed},
    },
};

use crate::charsets::{self, Charset};

use super::{
    TnefAttribute, TnefAttributeLevel as Level, TnefAttributeTag as Tag, TnefAttributeType as Type,
    TnefComplianceIssue, TnefComplianceLogger, TnefComplianceViolation as Violation, TnefDateTime,
    TnefDateTimeError, TnefReadError, TnefReaderOptions,
};

pub(super) fn decode_string(charset: Charset, bytes: &[u8]) -> Cow<'_, str> {
    // Eight-bit strings are usually NUL-terminated.
    let length = bytes
        .iter()
        .rposition(|&byte| byte != 0)
        .map_or(0, |index| index + 1);
    charset.decode_lossy(&bytes[..length]).0
}

enum Logger<'a> {
    None,
    Borrowed(&'a mut dyn TnefComplianceLogger),
    Shared(Rc<RefCell<&'a mut dyn TnefComplianceLogger>>),
}

impl<'a> Logger<'a> {
    fn is_some(&self) -> bool {
        !matches!(self, Self::None)
    }

    fn share(&mut self) -> Self {
        match self {
            Self::None => Self::None,
            Self::Shared(logger) => Self::Shared(Rc::clone(logger)),
            Self::Borrowed(_) => {
                let Self::Borrowed(logger) = std::mem::replace(self, Self::None) else {
                    unreachable!()
                };
                let logger = Rc::new(RefCell::new(logger));
                *self = Self::Shared(Rc::clone(&logger));
                Self::Shared(logger)
            }
        }
    }
}

impl TnefComplianceLogger for Logger<'_> {
    fn log(&mut self, issue: TnefComplianceIssue) -> io::Result<()> {
        match self {
            Self::None => Ok(()),
            Self::Borrowed(logger) => logger.log(issue),
            Self::Shared(logger) => logger.borrow_mut().log(issue),
        }
    }
}

// Only readers that actually open children allocate shared state. Atomics keep
// an ordinary root reader Send/Sync when its source is, without locking during
// callbacks. Exclusive parent borrowing serializes all budget updates.
struct SharedState {
    data_remaining: AtomicU64,
    reports: [AtomicUsize; 23],
    faulted: AtomicBool,
}

#[path = "property.rs"]
mod property;
pub use property::{TnefPropertyReader, TnefPropertyValueReader};

#[path = "embedded.rs"]
mod embedded;
pub use embedded::{TnefEmbeddedReader, TnefEmbeddedSource};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Claim {
    None,
    Raw,
    Scalar(i32),
    Date(Result<TnefDateTime, TnefDateTimeError>),
}

/// Forward-only, I/O-coupled TNEF attribute reader with 4 KiB read-ahead.
///
/// Construction performs no I/O. [`read_next`](Self::read_next) skips any
/// unread value and verifies its checksum before advancing. A value is
/// provisional until that advance succeeds; dropping a value stream or reader
/// does not drain it or validate its checksum.
///
/// Invalid signatures and impossible lengths end iteration. Other framing
/// defects are reported through the optional `*_with_logger` methods and
/// recovered as in MimeKit. Truncation is reported once; partial raw values
/// remain available. Source/logger failures are terminal and never retried
/// (except source `Interrupted`, retried before progress). No seeking is used.
///
/// Raw streams exclusively borrow the reader, so advancing while one is in use
/// is prevented by Rust rather than a runtime generation counter.
///
/// ```
/// use mimecraft::tnef::TnefReader;
/// let mut reader = TnefReader::new(b"\x78\x9f\x3e\x22\x01\x00".as_slice());
/// assert!(!reader.read_next()?);
/// assert_eq!(reader.legacy_key(), Some(1));
/// # Ok::<(), mimecraft::tnef::TnefReadError>(())
/// ```
pub struct TnefReader<R> {
    inner: R,
    options: TnefReaderOptions,
    input: [u8; 4096],
    index: usize,
    end: usize,
    position: u64,
    eof: bool,
    started: bool,
    done: bool,
    faulted: bool,
    truncated: bool,
    seen_attachment: bool,
    legacy_key: Option<u16>,
    attribute: Option<TnefAttribute>,
    remaining: u64,
    checksum: u16,
    claim: Claim,
    codepage: u32,
    charset: Charset,
    data_remaining: u64,
    reports: [usize; 23],
    property_tag: Option<super::TnefPropertyTag>,
    has_oem_codepage: bool,
    shared: Option<Arc<SharedState>>,
    depth: usize,
    skip_iid: bool,
    attach_method: i32,
}

impl<R: Read> TnefReader<R> {
    /// Creates a reader with default limits and Windows-1252 text decoding.
    pub fn new(inner: R) -> Self {
        Self::with_options(inner, &TnefReaderOptions::default())
            .expect("the default TNEF codepage is supported")
    }

    /// Copies options without reading the source.
    ///
    /// # Errors
    ///
    /// Rejects unsupported explicit initial codepages.
    pub fn with_options(inner: R, options: &TnefReaderOptions) -> Result<Self, TnefReadError> {
        let codepage = if options.default_codepage == 0 {
            1252
        } else {
            options.default_codepage
        };
        let charset = charsets::from_codepage(codepage)
            .ok_or(TnefReadError::UnsupportedCodepage(codepage))?;
        Ok(Self {
            inner,
            options: *options,
            input: [0; 4096],
            index: 0,
            end: 0,
            position: 0,
            eof: false,
            started: false,
            done: false,
            faulted: false,
            truncated: false,
            seen_attachment: false,
            legacy_key: None,
            attribute: None,
            remaining: 0,
            checksum: 0,
            claim: Claim::None,
            codepage,
            charset,
            data_remaining: options.max_total_data_bytes,
            reports: [0; 23],
            property_tag: None,
            has_oem_codepage: false,
            shared: None,
            depth: 0,
            skip_iid: false,
            attach_method: 1,
        })
    }

    /// Returns the copied configuration.
    pub fn options(&self) -> &TnefReaderOptions {
        &self.options
    }

    /// Returns the legacy key after the complete stream header is consumed.
    pub fn legacy_key(&self) -> Option<u16> {
        self.legacy_key
    }

    /// Returns current metadata; cleared at the end of iteration.
    pub fn attribute(&self) -> Option<&TnefAttribute> {
        self.attribute.as_ref()
    }

    /// Returns the current codepage for 8-bit strings.
    pub fn codepage(&self) -> u32 {
        self.codepage
    }

    /// Returns the consumed offset in the outermost stream, excluding read-ahead.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Returns unread bytes of the current declared value.
    pub fn value_remaining(&self) -> u64 {
        self.remaining
    }

    /// Returns whether a source/logger/arithmetic error has made reads terminal.
    pub fn is_faulted(&self) -> bool {
        self.faulted
            || self
                .shared
                .as_ref()
                .is_some_and(|state| state.faulted.load(Relaxed))
    }

    /// Returns embedded-message depth; the outermost reader is at depth zero.
    pub fn depth(&self) -> usize {
        self.depth
    }

    fn fault(&mut self) {
        self.faulted = true;
        if let Some(state) = &self.shared {
            state.faulted.store(true, Relaxed);
        }
    }

    fn shared_state(&mut self) -> Arc<SharedState> {
        Arc::clone(self.shared.get_or_insert_with(|| {
            Arc::new(SharedState {
                data_remaining: AtomicU64::new(self.data_remaining),
                reports: std::array::from_fn(|i| AtomicUsize::new(self.reports[i])),
                faulted: AtomicBool::new(self.faulted),
            })
        }))
    }

    fn data_remaining(&self) -> u64 {
        self.shared.as_ref().map_or(self.data_remaining, |state| {
            state.data_remaining.load(Relaxed)
        })
    }

    fn set_data_remaining(&mut self, remaining: u64) {
        if let Some(state) = &self.shared {
            state.data_remaining.store(remaining, Relaxed);
        } else {
            self.data_remaining = remaining;
        }
    }

    /// Returns whether iteration has ended normally or after malformed framing.
    /// A source/logger error is instead indicated by [`is_faulted`](Self::is_faulted).
    pub fn is_finished(&self) -> bool {
        self.done
    }

    /// Returns whether premature EOF has been observed.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Returns the source without bypassing read-ahead.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Returns the source and unread buffered bytes. Replay those bytes first.
    /// Does not drain or validate a pending value.
    pub fn into_parts(self) -> (R, Vec<u8>) {
        (self.inner, self.input[self.index..self.end].to_vec())
    }

    /// Advances, discarding optional diagnostics.
    ///
    /// # Errors
    ///
    /// Returns source, position-overflow, or prior-failure errors.
    pub fn read_next(&mut self) -> Result<bool, TnefReadError> {
        self.run(|reader| reader.next_core(&mut Logger::None))
    }

    /// Advances, reporting framing/checksum issues under the shared report cap.
    ///
    /// # Errors
    ///
    /// Source/logger errors abort and fault the reader.
    pub fn read_next_with_logger(
        &mut self,
        logger: &mut dyn TnefComplianceLogger,
    ) -> Result<bool, TnefReadError> {
        self.run(|reader| reader.next_core(&mut Logger::Borrowed(logger)))
    }

    /// Claims the raw value for bounded streaming without an owned-value limit.
    ///
    /// # Errors
    ///
    /// Requires an unclaimed current attribute and a nonfaulted reader.
    pub fn open_value(&mut self) -> Result<TnefValueReader<'_, R>, TnefReadError> {
        self.open_core(Logger::None)
    }

    /// Claims a MAPI-properties, attachment-properties, or recipient-table
    /// attribute. Dropping the property reader performs no I/O; parent advance
    /// skips the remainder and verifies the attribute checksum.
    ///
    /// # Errors
    ///
    /// Requires an unclaimed property-bearing attribute and a nonfaulted reader.
    pub fn open_properties(&mut self) -> Result<TnefPropertyReader<'_, R>, TnefReadError> {
        self.properties_core(Logger::None)
    }

    /// Claims a property reader with a logger for its entire borrowed lifetime.
    ///
    /// # Errors
    ///
    /// As [`open_properties`](Self::open_properties).
    pub fn open_properties_with_logger<'a>(
        &'a mut self,
        logger: &'a mut dyn TnefComplianceLogger,
    ) -> Result<TnefPropertyReader<'a, R>, TnefReadError> {
        self.properties_core(Logger::Borrowed(logger))
    }

    fn properties_core<'a>(
        &'a mut self,
        logger: Logger<'a>,
    ) -> Result<TnefPropertyReader<'a, R>, TnefReadError> {
        let tag = self.check_unclaimed()?.tag;
        if !matches!(
            tag,
            Tag::MAPI_PROPERTIES | Tag::ATTACHMENT | Tag::RECIPIENT_TABLE
        ) {
            return Err(TnefReadError::InvalidValueType(tag));
        }
        self.claim = Claim::Raw;
        Ok(TnefPropertyReader::new(
            self,
            logger,
            tag == Tag::RECIPIENT_TABLE,
        ))
    }

    /// Claims a raw stream that reports truncation through the supplied logger.
    /// Checksum reports are emitted when the parent reader next advances.
    ///
    /// # Errors
    ///
    /// Requires an unclaimed current attribute and a nonfaulted reader.
    pub fn open_value_with_logger<'a>(
        &'a mut self,
        logger: &'a mut dyn TnefComplianceLogger,
    ) -> Result<TnefValueReader<'a, R>, TnefReadError> {
        self.open_core(Logger::Borrowed(logger))
    }

    fn open_core<'a>(
        &'a mut self,
        logger: Logger<'a>,
    ) -> Result<TnefValueReader<'a, R>, TnefReadError> {
        self.check_unclaimed()?;
        self.claim = Claim::Raw;
        Ok(TnefValueReader {
            reader: self,
            logger,
        })
    }

    /// Collects a raw value once, bounded by both configured owned-byte limits.
    /// Partial bytes are returned on truncation, with [`is_truncated`](Self::is_truncated) set.
    ///
    /// # Errors
    ///
    /// A limit error consumes and claims nothing, allowing a streaming retry.
    /// Source errors fault the reader. Other errors indicate invalid API state.
    pub fn read_value_bytes(&mut self) -> Result<Vec<u8>, TnefReadError> {
        self.bytes_core(&mut Logger::None)
    }

    /// Collects a raw value, reporting limit failures and truncation.
    ///
    /// # Errors
    ///
    /// As [`read_value_bytes`](Self::read_value_bytes); logger failures also fault the reader.
    pub fn read_value_bytes_with_logger(
        &mut self,
        logger: &mut dyn TnefComplianceLogger,
    ) -> Result<Vec<u8>, TnefReadError> {
        self.bytes_core(&mut Logger::Borrowed(logger))
    }

    /// Reads a string with the current codepage, trimming trailing NUL bytes.
    /// Invalid byte sequences are replaced as in MimeKit's default encodings.
    ///
    /// # Errors
    ///
    /// Requires a string-compatible tag; otherwise shares owned-value errors.
    pub fn read_value_string(&mut self) -> Result<String, TnefReadError> {
        self.string_core(&mut Logger::None)
    }

    /// Reads a string with optional framing/limit diagnostics.
    ///
    /// # Errors
    ///
    /// As [`read_value_string`](Self::read_value_string), plus logger failures.
    pub fn read_value_string_with_logger(
        &mut self,
        logger: &mut dyn TnefComplianceLogger,
    ) -> Result<String, TnefReadError> {
        self.string_core(&mut Logger::Borrowed(logger))
    }

    /// Reads a two- or four-byte integer. Repeated scalar reads return the cache.
    /// Shorts are sign-extended, words zero-extended, and DWORDs retain their
    /// bits in an `i32`. Malformed short values yield zero and a diagnostic.
    ///
    /// # Errors
    ///
    /// Requires an integer tag not already consumed in another way.
    pub fn read_value_i32(&mut self) -> Result<i32, TnefReadError> {
        self.scalar_core(&mut Logger::None)
    }

    /// Reads an integer, reporting malformed values and truncation.
    ///
    /// # Errors
    ///
    /// As [`read_value_i32`](Self::read_value_i32), plus logger failures.
    pub fn read_value_i32_with_logger(
        &mut self,
        logger: &mut dyn TnefComplianceLogger,
    ) -> Result<i32, TnefReadError> {
        self.scalar_core(&mut Logger::Borrowed(logger))
    }

    /// Reads and caches a fourteen-byte attribute date with an unspecified zone.
    /// The weekday is ignored. Repeated reads neither consume nor report again.
    ///
    /// # Errors
    ///
    /// Invalid/incomplete dates return `InvalidDate`, not a minimum-date fallback.
    /// Wrong types and claimed values are rejected. Source/logger failures fault
    /// the reader; invalid dates do not. Fixed date reads use no owned-byte budget.
    pub fn read_value_datetime(&mut self) -> Result<TnefDateTime, TnefReadError> {
        self.date_core(&mut Logger::None)
    }

    /// Reads an attribute date with diagnostics for invalid dates and truncation.
    ///
    /// # Errors
    ///
    /// As [`read_value_datetime`](Self::read_value_datetime).
    pub fn read_value_datetime_with_logger(
        &mut self,
        logger: &mut dyn TnefComplianceLogger,
    ) -> Result<TnefDateTime, TnefReadError> {
        self.date_core(&mut Logger::Borrowed(logger))
    }

    fn check_attribute(&self) -> Result<TnefAttribute, TnefReadError> {
        if self.is_faulted() {
            return Err(TnefReadError::Faulted);
        }
        self.attribute.ok_or(TnefReadError::NoAttribute)
    }

    fn check_unclaimed(&self) -> Result<TnefAttribute, TnefReadError> {
        let attribute = self.check_attribute()?;
        if self.claim != Claim::None {
            return Err(TnefReadError::ValueAlreadyRead);
        }
        Ok(attribute)
    }

    fn run<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, TnefReadError>,
    ) -> Result<T, TnefReadError> {
        if self.is_faulted() {
            return Err(TnefReadError::Faulted);
        }
        let result = operation(self);
        if result.is_err() {
            self.fault();
        }
        result
    }

    fn log(
        &mut self,
        logger: &mut Logger<'_>,
        violation: Violation,
        offset: u64,
    ) -> Result<(), TnefReadError> {
        if logger.is_some() {
            let limit = self.options.max_issues_per_violation;
            if limit > 0 {
                if let Some(state) = &self.shared {
                    let count = &state.reports[violation as usize];
                    let value = count.load(Relaxed);
                    if value >= limit {
                        return Ok(());
                    }
                    count.store(value + 1, Relaxed);
                } else {
                    let count = &mut self.reports[violation as usize];
                    if *count >= limit {
                        return Ok(());
                    }
                    *count += 1;
                }
            }
            logger.log(TnefComplianceIssue {
                violation,
                stream_offset: offset,
                depth: self.depth,
                attribute_tag: self.attribute.map(|attribute| attribute.tag),
                property_tag: self.property_tag,
            })?;
        }
        Ok(())
    }

    fn truncated(&mut self, logger: &mut Logger<'_>) -> Result<(), TnefReadError> {
        // Once the stream is truncated, nothing further can be read.
        if !self.truncated {
            self.truncated = true;
            self.log(logger, Violation::TruncatedStream, self.position)?;
        }
        Ok(())
    }

    fn advance(&mut self, count: usize) -> Result<(), TnefReadError> {
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or(TnefReadError::PositionOverflow)?;
        self.index += count;
        Ok(())
    }

    fn read_source(inner: &mut R, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            match inner.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }

    // Move buffered bytes to the start so lookahead works even with short reads.
    fn fill(&mut self, count: usize) -> Result<bool, TnefReadError> {
        debug_assert!(count <= self.input.len());
        if self.end - self.index >= count {
            return Ok(true);
        }
        self.input.copy_within(self.index..self.end, 0);
        self.end -= self.index;
        self.index = 0;
        while self.end < count {
            if self.eof {
                return Ok(false);
            }
            let read = Self::read_source(&mut self.inner, &mut self.input[self.end..])?;
            if read == 0 {
                self.eof = true;
                return Ok(false);
            }
            self.end += read;
        }
        Ok(true)
    }

    fn end_iteration(&mut self) -> bool {
        self.done = true;
        self.attribute = None;
        self.remaining = 0;
        false
    }

    fn next_core(&mut self, logger: &mut Logger<'_>) -> Result<bool, TnefReadError> {
        if self.done {
            return Ok(false);
        }
        if !self.started {
            self.started = true;
            if self.skip_iid {
                // An embedded message begins with the 16-byte IID of IMessage.
                if !self.fill(16)? {
                    self.truncated(logger)?;
                    return Ok(self.end_iteration());
                }
                self.advance(16)?;
            }
            if !self.fill(4)? {
                self.truncated(logger)?;
                return Ok(self.end_iteration());
            }
            let signature =
                u32::from_le_bytes(self.input[self.index..self.index + 4].try_into().unwrap());
            if signature != 0x223e9f78 {
                // A wrong signature is not TNEF; nothing meaningful remains.
                self.log(logger, Violation::InvalidSignature, self.position)?;
                return Ok(self.end_iteration());
            }
            self.advance(4)?;
            if !self.fill(2)? {
                self.truncated(logger)?;
                return Ok(self.end_iteration());
            }
            self.legacy_key = Some(u16::from_le_bytes(
                self.input[self.index..self.index + 2].try_into().unwrap(),
            ));
            self.advance(2)?;
        } else if !self.finish_attribute(logger)? {
            return Ok(self.end_iteration());
        }
        self.attribute = None;
        self.claim = Claim::None;
        self.checksum = 0;
        // EOF between attributes is the normal way for a TNEF stream to end.
        if !self.fill(1)? {
            return Ok(self.end_iteration());
        }
        if !self.fill(9)? {
            self.truncated(logger)?;
            return Ok(self.end_iteration());
        }
        let bytes = &self.input[self.index..self.index + 9];
        let level = Level::from(bytes[0]);
        let tag = Tag(u32::from_le_bytes(bytes[1..5].try_into().unwrap()));
        let length = i32::from_le_bytes(bytes[5..9].try_into().unwrap());
        let offset = self.position;
        self.advance(9)?;
        self.attribute = Some(TnefAttribute {
            level,
            tag,
            length: length.max(0) as u32,
            offset,
        });
        match level {
            // Once attachment attributes begin, message-level attributes
            // cannot legally resume.
            Level::Attachment => self.seen_attachment = true,
            Level::Message if self.seen_attachment => {
                self.log(logger, Violation::MessageAttributeAfterAttachment, offset)?;
            }
            Level::Unknown(_) => self.log(logger, Violation::InvalidAttributeLevel, offset)?,
            _ => {}
        }
        if length < 0 {
            // The DWORD exceeds i32::MAX. We cannot locate the next attribute.
            self.log(logger, Violation::InvalidAttributeLength, offset)?;
            return Ok(self.end_iteration());
        }
        self.remaining = length as u64;
        self.check_tag(logger)?;
        Ok(true)
    }

    fn check_tag(&mut self, logger: &mut Logger<'_>) -> Result<(), TnefReadError> {
        let attribute = self.attribute.unwrap();
        let TnefAttribute {
            tag,
            length,
            offset,
            level,
        } = attribute;
        match tag {
            Tag::ATTACH_RENDER_DATA => {
                // attAttachRenderData marks the start of a new attachment.
                self.attach_method = 1;
            }
            Tag::MESSAGE_CLASS | Tag::ORIGINAL_MESSAGE_CLASS => {
                // MS-OXCMSG limits the class to 255 bytes. A single final NUL
                // is allowed; a short source is truncation, not an invalid class.
                if length == 0 || length > 255 {
                    self.log(logger, Violation::InvalidMessageClass, offset)?;
                } else if self.fill(length as usize)? {
                    let bytes = &self.input[self.index..self.index + length as usize];
                    if bytes.iter().enumerate().any(|(index, &byte)| {
                        !(0x20..=0x7e).contains(&byte) && !(byte == 0 && index + 1 == bytes.len())
                    }) {
                        self.log(logger, Violation::InvalidMessageClass, offset)?;
                    }
                }
            }
            Tag::OEM_CODEPAGE | Tag::TNEF_VERSION => {
                if length < 4 {
                    self.log(logger, Violation::InvalidAttributeValue, offset)?;
                } else if self.fill(4)? {
                    let value = u32::from_le_bytes(
                        self.input[self.index..self.index + 4].try_into().unwrap(),
                    );
                    if tag == Tag::TNEF_VERSION {
                        if value != 0x00010000 {
                            self.log(logger, Violation::UnsupportedVersion, offset)?;
                        }
                    } else if value != 0 {
                        // The nonzero OEM codepage takes precedence for
                        // subsequent 8-bit strings. Keep the current charset
                        // if the declared codepage cannot be resolved.
                        if let Some(charset) = charsets::from_codepage(value) {
                            self.codepage = value;
                            self.charset = charset;
                            self.has_oem_codepage = true;
                        } else {
                            self.log(logger, Violation::InvalidMessageCodepage, offset)?;
                        }
                    }
                }
            }
            _ if !tag.is_known() => {
                self.log(logger, Violation::UnknownAttribute, offset)?;
                return Ok(());
            }
            _ => {}
        }
        // Null is allowed at either level; unknown levels were already reported.
        if tag != Tag::NULL
            && !matches!(level, Level::Unknown(_))
            && tag.is_attachment() != (level == Level::Attachment)
        {
            self.log(logger, Violation::AttributeLevelMismatch, offset)?;
        }
        Ok(())
    }

    fn take_value(&mut self, count: usize) -> Result<(), TnefReadError> {
        self.checksum = checksum(self.checksum, &self.input[self.index..self.index + count]);
        self.advance(count)?;
        self.remaining -= count as u64;
        Ok(())
    }

    fn read_raw(
        &mut self,
        buffer: &mut [u8],
        logger: &mut Logger<'_>,
    ) -> Result<usize, TnefReadError> {
        let count = buffer.len().min(self.remaining as usize);
        if count == 0 || self.truncated {
            return Ok(0);
        }
        if self.index == self.end && count >= self.input.len() && !self.eof {
            // Large reads bypass read-ahead and go directly to the caller.
            let read = Self::read_source(&mut self.inner, &mut buffer[..count])?;
            self.position = self
                .position
                .checked_add(read as u64)
                .ok_or(TnefReadError::PositionOverflow)?;
            self.remaining -= read as u64;
            self.checksum = checksum(self.checksum, &buffer[..read]);
            if read == 0 {
                self.eof = true;
                self.truncated(logger)?;
            }
            return Ok(read);
        }
        if !self.fill(1)? {
            self.truncated(logger)?;
            return Ok(0);
        }
        let count = count.min(self.end - self.index);
        buffer[..count].copy_from_slice(&self.input[self.index..self.index + count]);
        self.take_value(count)?;
        Ok(count)
    }

    fn finish_attribute(&mut self, logger: &mut Logger<'_>) -> Result<bool, TnefReadError> {
        if self.truncated {
            return Ok(false);
        }
        // Skipping still includes every value byte in the checksum.
        while self.remaining > 0 {
            if !self.fill(1)? {
                self.truncated(logger)?;
                return Ok(false);
            }
            self.take_value((self.end - self.index).min(self.remaining as usize))?;
        }
        if !self.fill(2)? {
            self.truncated(logger)?;
            return Ok(false);
        }
        let expected =
            u16::from_le_bytes(self.input[self.index..self.index + 2].try_into().unwrap());
        self.advance(2)?;
        if expected != self.checksum {
            self.log(
                logger,
                Violation::AttributeChecksumMismatch,
                self.attribute.unwrap().offset,
            )?;
        }
        Ok(true)
    }

    fn bytes_core(&mut self, logger: &mut Logger<'_>) -> Result<Vec<u8>, TnefReadError> {
        self.check_unclaimed()?;
        let output = self.collect_bytes(self.remaining, logger)?;
        self.claim = Claim::Raw;
        Ok(output)
    }

    fn collect_bytes(
        &mut self,
        count: u64,
        logger: &mut Logger<'_>,
    ) -> Result<Vec<u8>, TnefReadError> {
        debug_assert!(count <= self.remaining);
        if count > self.options.max_value_length as u64 || count > self.data_remaining() {
            self.run(|reader| {
                reader.log(logger, Violation::DataSizeLimitExceeded, reader.position)
            })?;
            return Err(TnefReadError::ValueLimitExceeded);
        }
        self.set_data_remaining(self.data_remaining() - count);
        self.run(|reader| {
            // Do not trust a declared length with a large up-front allocation.
            // Grow only as actual bytes arrive, even for nonseekable sources.
            let mut output = Vec::with_capacity((count as usize).min(64 * 1024));
            let mut buffer = [0; 4096];
            while (output.len() as u64) < count {
                let length = buffer.len().min((count - output.len() as u64) as usize);
                let read = reader.read_raw(&mut buffer[..length], logger)?;
                if read == 0 {
                    break;
                }
                output.extend_from_slice(&buffer[..read]);
            }
            // Refund advertised bytes that were never available on truncation.
            reader.set_data_remaining(reader.data_remaining() + count - output.len() as u64);
            Ok(output)
        })
    }

    fn string_core(&mut self, logger: &mut Logger<'_>) -> Result<String, TnefReadError> {
        let tag = self.check_unclaimed()?.tag;
        // MS-OXTNEF declares message classes as Word, but their values are
        // NUL-terminated strings rather than integers.
        if !matches!(
            tag.attribute_type(),
            Type::Triples | Type::String | Type::Text | Type::Byte
        ) && !matches!(tag, Tag::MESSAGE_CLASS | Tag::ORIGINAL_MESSAGE_CLASS)
        {
            return Err(TnefReadError::InvalidValueType(tag));
        }
        let bytes = self.bytes_core(logger)?;
        Ok(decode_string(self.charset, &bytes).into_owned())
    }

    fn load_scalar(
        &mut self,
        attribute: TnefAttribute,
        width: usize,
        logger: &mut Logger<'_>,
    ) -> Result<([u8; 14], usize), TnefReadError> {
        let count = (attribute.length as usize).min(width);
        self.fill(count)?;
        let read = count.min(self.end - self.index);
        let mut bytes = [0; 14];
        bytes[..read].copy_from_slice(&self.input[self.index..self.index + read]);
        self.take_value(read)?;
        if read < count {
            self.truncated(logger)?;
        }
        if (attribute.length as usize) < width {
            self.log(logger, Violation::InvalidAttributeValue, attribute.offset)?;
        }
        Ok((bytes, read))
    }

    fn scalar_core(&mut self, logger: &mut Logger<'_>) -> Result<i32, TnefReadError> {
        let attribute = self.check_attribute()?;
        let kind = attribute.tag.attribute_type();
        let width = match kind {
            Type::Short | Type::Word => 2,
            Type::Long | Type::DWord => 4,
            _ => return Err(TnefReadError::InvalidValueType(attribute.tag)),
        };
        match self.claim {
            Claim::Scalar(value) => return Ok(value),
            Claim::Raw | Claim::Date(_) => return Err(TnefReadError::ValueAlreadyRead),
            Claim::None => {}
        }
        self.run(|reader| {
            let (bytes, read) = reader.load_scalar(attribute, width, logger)?;
            let value = if read < width {
                0
            } else {
                match kind {
                    Type::Short => i32::from(i16::from_le_bytes(bytes[..2].try_into().unwrap())),
                    Type::Word => i32::from(u16::from_le_bytes(bytes[..2].try_into().unwrap())),
                    _ => i32::from_le_bytes(bytes[..4].try_into().unwrap()),
                }
            };
            reader.claim = Claim::Scalar(value);
            Ok(value)
        })
    }

    fn date_core(&mut self, logger: &mut Logger<'_>) -> Result<TnefDateTime, TnefReadError> {
        let attribute = self.check_attribute()?;
        if attribute.tag.attribute_type() != Type::Date {
            return Err(TnefReadError::InvalidValueType(attribute.tag));
        }
        match self.claim {
            Claim::Date(value) => return value.map_err(TnefReadError::InvalidDate),
            Claim::None => {}
            _ => return Err(TnefReadError::ValueAlreadyRead),
        }
        let date = self.run(|reader| {
            let (bytes, read) = reader.load_scalar(attribute, 14, logger)?;
            let date = if read < 14 {
                Err(TnefDateTimeError::IncompleteValue)
            } else {
                TnefDateTime::from_attribute(&bytes)
            };
            if read == 14 && date.is_err() {
                reader.log(logger, Violation::InvalidDate, attribute.offset)?;
            }
            reader.claim = Claim::Date(date);
            // Invalid dates are recoverable; only I/O/logger/arithmetic errors
            // escape run() and fault the parent.
            Ok(date)
        })?;
        date.map_err(TnefReadError::InvalidDate)
    }
}

/// Exclusively borrowed raw value stream. EOF cannot consume its checksum or
/// the next attribute. Dropping leaves unread bytes for the next parent advance.
pub struct TnefValueReader<'a, R> {
    reader: &'a mut TnefReader<R>,
    logger: Logger<'a>,
}

impl<R: Read> Read for TnefValueReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reader
            .run(|reader| reader.read_raw(buffer, &mut self.logger))
            .map_err(|error| match error {
                TnefReadError::Io(error) => error,
                error => io::Error::other(error),
            })
    }
}

fn checksum(previous: u16, bytes: &[u8]) -> u16 {
    // Addition modulo 65536 is associative. Accumulate at full width and
    // truncate once per bounded block, not after each byte. A 64 KiB block
    // sums to at most 65536 * 255, so no u32 accumulator can overflow.
    bytes.chunks(65536).fold(previous, |sum, block| {
        let block_sum: u32 = block.iter().map(|&byte| u32::from(byte)).sum();
        sum.wrapping_add(block_sum as u16)
    })
}

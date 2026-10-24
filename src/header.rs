// Adapted from MimeKit's Header.cs, MimeReader.cs, and ParserOptions.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, BufRead},
    num::NonZeroUsize,
    str::FromStr,
};

use crate::{
    AddressList, AddressParseError, AddressParseOptions, ContentDisposition,
    ContentHeaderParseError, ContentType, HeaderId, ParameterParseOptions,
    rfc2047::{self, DecodeOptions, DecodedHeader},
    utils::unfold_in_place,
};

/// A byte-preserving header record, including its folded continuation lines.
///
/// [`HeaderParser`] preserves field spelling, whitespace, line endings, and
/// non-UTF-8 value bytes exactly. Standalone [`parse`](Self::parse) instead
/// requires a field/colon boundary and adds a terminal CRLF when LF is absent.
/// [`decode_value`](Self::decode_value) returns unfolded Unicode text with
/// charset recovery diagnostics, without changing the stored record.
/// Structured parsing methods operate on raw value bytes, without display
/// unfolding or a prior decoding pass. Typed models can create replacement
/// records via their `to_header` methods; records themselves remain immutable.
///
/// Malformed records are retained and explicitly marked by [`is_invalid`](Self::is_invalid).
/// That flag checks only the field/colon syntax, not value syntax or compliance.
/// Like MimeKit, an empty name followed by a colon is not marked invalid.
#[derive(Clone, Debug, Eq)]
pub struct Header {
    raw: Vec<u8>,
    field: Option<(usize, usize)>,
    id: HeaderId,
    offset: Option<u64>,
}

impl PartialEq for Header {
    fn eq(&self, other: &Self) -> bool {
        // Source position is metadata, not part of header value
        // equality. A loaded record must still equal the same authored bytes.
        self.raw == other.raw && self.field == other.field && self.id == other.id
    }
}

impl Header {
    /// Creates a checked unstructured header, such as Subject or Comments.
    ///
    /// Uses [`Self::from_unstructured_with_options`] with default options.
    ///
    /// # Errors
    ///
    /// Returns [`crate::UnstructuredHeaderFormatError`] without a partial record.
    pub fn from_unstructured(
        field: &str,
        value: &str,
    ) -> Result<Self, crate::UnstructuredHeaderFormatError> {
        Self::from_unstructured_with_options(
            field,
            value,
            &crate::UnstructuredHeaderFormatOptions::default(),
        )
    }

    /// Creates an ASCII unstructured header with explicit formatting options.
    ///
    /// The field must be nonempty printable ASCII without a colon. Its spelling
    /// is preserved. Like MimeKit's value assignment, trims Unicode whitespace
    /// and removes internal CR/LF before encoding; other controls are encoded,
    /// not silently removed. The value is literal text: encoded-word lookalikes
    /// are encoded again so readers do not reinterpret them.
    ///
    /// Uses [`rfc2047::encode_text_with_options`]'s whole-value encoding policy,
    /// not MimeKit's mixed plain/encoded-word heuristics. Folds at spaces without
    /// splitting words or changing interior whitespace. Empty normalized text
    /// creates `field:` plus the selected newline, not an absent field.
    ///
    /// This does not dispatch on the field name or validate structured grammar.
    /// Use address/content/identifier formatters for those fields. Decoded
    /// controls remain controls: this is safe header framing, not sanitization
    /// for terminals, HTML, or other display contexts.
    ///
    /// # Errors
    ///
    /// Returns [`crate::UnstructuredHeaderFormatError`] for invalid fields or
    /// options, an oversized field name, or text that cannot be encoded.
    /// Encoding offsets refer to the trimmed, unfolded text.
    ///
    /// ```
    /// use mimecraft::Header;
    ///
    /// let header = Header::from_unstructured("Subject", " Hello Ren\u{e9}e ")?;
    /// assert!(header.raw().is_ascii());
    /// assert_eq!(header.decode_value().text, "Hello Ren\u{e9}e");
    /// # Ok::<(), mimecraft::UnstructuredHeaderFormatError>(())
    /// ```
    pub fn from_unstructured_with_options(
        field: &str,
        value: &str,
        options: &crate::UnstructuredHeaderFormatOptions,
    ) -> Result<Self, crate::UnstructuredHeaderFormatError> {
        crate::unstructured_header_format::format_header(field, value, options)
            .map(Self::from_formatted)
    }

    /// Parses a standalone header from the entire byte slice.
    ///
    /// Matches MimeKit's public header parser: requires an ASCII field followed
    /// by optional spaces/tabs and a colon, but accepts an empty field name.
    /// Original bytes are retained, except that CRLF is appended if the input
    /// does not end in LF. An existing LF ending is not converted; a terminal
    /// bare CR is retained before the appended CRLF.
    ///
    /// Unlike MimeKit's platform-dependent default, the appended newline is
    /// always CRLF, consistent with this crate's other defaults.
    ///
    /// **This is not a header-block parser or authoring validator.** Everything
    /// after the first field's colon is its value, including embedded header-like
    /// lines, blank lines, controls, and arbitrary bytes. No folding syntax or
    /// value grammar is validated. The caller must isolate one header and bound
    /// its size; this method does not apply the streaming parser's record limit.
    /// Use [`HeaderParser`] for block boundaries and exact byte preservation.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderParseError`] when no colon follows the field and optional
    /// blanks. The offset identifies where a colon was expected, relative to
    /// this input slice.
    ///
    /// ```
    /// use mimecraft::{Header, HeaderId};
    ///
    /// let header = Header::parse(b"Subject: =?utf-8?q?caf=C3=A9?=")?;
    /// assert_eq!(header.id(), HeaderId::Subject);
    /// assert!(header.raw().ends_with(b"\r\n"));
    /// assert_eq!(header.decode_value().text, "caf\u{e9}");
    /// # Ok::<(), mimecraft::HeaderParseError>(())
    /// ```
    pub fn parse(input: &[u8]) -> Result<Self, HeaderParseError> {
        let field = parse_field(input)?;
        let mut raw = input.to_vec();
        // MimeKit's standalone Parse/TryParse APIs canonicalize the value by
        // appending a newline when missing; the stream parser must not do so.
        if raw.last() != Some(&b'\n') {
            raw.extend_from_slice(b"\r\n");
        }
        Ok(Self::from_parts(raw, Some(field)))
    }

    pub(crate) fn from_formatted(raw: String) -> Self {
        let field = parse_field(raw.as_bytes())
            .expect("typed header formatters emit a valid field and colon");
        Self::from_parts(raw.into_bytes(), Some(field))
    }

    fn from_raw(raw: Vec<u8>) -> Self {
        // Without a valid field/colon boundary, keep the whole record as its
        // raw field and expose an empty value, matching MimeKit's invalid headers.
        let field = parse_field(&raw).ok();
        Self::from_parts(raw, field)
    }

    fn from_parts(raw: Vec<u8>, field: Option<(usize, usize)>) -> Self {
        let mut header = Self {
            raw,
            field,
            id: HeaderId::Unknown,
            offset: None,
        };
        header.id = header
            .field()
            .map_or(HeaderId::Unknown, HeaderId::from_name);
        header
    }

    /// Returns the cached identifier for this record's field name.
    ///
    /// Unknown extension names, empty names, and malformed records return
    /// [`HeaderId::Unknown`]. Use [`is_invalid`](Self::is_invalid) to distinguish
    /// malformed records; identifying a name does not validate its value.
    /// Original spelling remains available through [`field`](Self::field).
    pub fn id(&self) -> HeaderId {
        self.id
    }

    /// Returns the byte offset where this header began in its parsed stream.
    ///
    /// Offsets count original bytes, including prior headers, folding, and MIME
    /// framing, not indices in a header list. Non-seeking stream loaders count
    /// from the start of their input; nested entities share that origin.
    /// Standalone parsing and authored headers return `None`. Clones retain
    /// offsets, but replacing a record does not assign the old record's offset.
    /// Partial-message reassembly clears offsets of combined root headers.
    /// Offsets do not participate in header value equality.
    pub fn offset(&self) -> Option<u64> {
        self.offset
    }

    pub(crate) fn clear_offset(&mut self) {
        self.offset = None;
    }

    /// Returns the ASCII field name, excluding blanks before the colon.
    ///
    /// Returns `None` for malformed records and `Some("")` for an empty name.
    /// Original capitalization is retained; use [`matches_field`](Self::matches_field)
    /// for case-insensitive lookup.
    pub fn field(&self) -> Option<&str> {
        self.field.map(|(length, _)| {
            std::str::from_utf8(&self.raw[..length]).expect("field names contain only ASCII")
        })
    }

    /// Tests a field name using ASCII case-insensitive comparison.
    ///
    /// Malformed records never match, including against an empty name.
    pub fn matches_field(&self, field: &str) -> bool {
        self.field()
            .is_some_and(|name| name.eq_ignore_ascii_case(field))
    }

    /// Returns whether the parsed field name starts with `Content-`, ignoring case.
    ///
    /// Includes unknown extension fields, not just known MIME headers.
    /// Malformed records return false. This classifies the name only; it does
    /// not parse the value or validate where the header appears.
    pub fn is_content_header(&self) -> bool {
        self.field()
            .and_then(|field| field.get(..8))
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Content-"))
    }

    /// Returns whether the field name and colon could not be parsed.
    pub fn is_invalid(&self) -> bool {
        self.field.is_none()
    }

    /// Returns the stored record, without a separately tracked body separator.
    ///
    /// Includes any terminating newline. [`HeaderParser`] keeps input exactly;
    /// [`parse`](Self::parse) appends CRLF when a terminal LF is absent.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// Returns bytes before the colon, including any trailing spaces or tabs.
    ///
    /// For malformed records, returns the entire record, matching MimeKit's
    /// storage convention for invalid headers.
    pub fn raw_field(&self) -> &[u8] {
        let end = self.field.map_or(self.raw.len(), |(_, colon)| colon);
        &self.raw[..end]
    }

    /// Returns bytes after the colon, including whitespace and folded newlines.
    ///
    /// Returns an empty slice for malformed records. No encoded-word decoding,
    /// charset conversion, trimming, or unfolding is performed.
    pub fn raw_value(&self) -> &[u8] {
        let start = self.field.map_or(self.raw.len(), |(_, colon)| colon + 1);
        &self.raw[start..]
    }

    /// Parses the raw value as an address list, without decoding it as display text.
    ///
    /// The caller selects the grammar: the field name is not checked, so this
    /// also works for extension address fields. No field-specific address-count
    /// or group rules are imposed. Syntax offsets refer to [`Self::raw_value`];
    /// name-decoding diagnostics retain the address parser's preprocessing rules.
    /// Parsing does not change this header or cache a mutable typed value.
    ///
    /// ```
    /// use mimecraft::Header;
    ///
    /// let header = Header::parse(b"To: =?utf-8?q?Doe=2C_Jane?= <jane@example.com>")?;
    /// let addresses = header.parse_addresses()?;
    /// assert_eq!(addresses.len(), 1);
    /// assert_eq!(addresses.mailboxes().next().unwrap().display_name(), "Doe, Jane");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Uses [`AddressList::parse`]'s complete-value rules, including errors for
    /// empty/CFWS-only values (even Bcc) and malformed members. A malformed raw
    /// header has an empty raw value and consequently also fails.
    pub fn parse_addresses(&self) -> Result<AddressList, AddressParseError> {
        self.parse_addresses_with_options(&AddressParseOptions::default())
    }

    /// Parses raw address syntax with explicit compatibility and name options.
    ///
    /// See [`Self::parse_addresses`]. No preliminary decoding or unfolding occurs.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] with syntax offsets relative to the raw value.
    pub fn parse_addresses_with_options(
        &self,
        options: &AddressParseOptions,
    ) -> Result<AddressList, AddressParseError> {
        AddressList::parse_with_options(self.raw_value(), options)
    }

    /// Parses the raw value as Content-Type with default parameter options.
    ///
    /// The field name is not checked. Parameter diagnostics and syntax offsets
    /// are relative to [`Self::raw_value`], including its initial whitespace.
    /// No preliminary text decoding, unfolding, fallback media type, or mutation
    /// is performed.
    ///
    /// # Errors
    ///
    /// Returns [`ContentHeaderParseError`] for malformed or empty value syntax.
    pub fn parse_content_type(&self) -> Result<ContentType, ContentHeaderParseError> {
        self.parse_content_type_with_options(&ParameterParseOptions::default())
    }

    /// Parses raw Content-Type syntax with explicit parameter options.
    ///
    /// See [`Self::parse_content_type`]. Options affect parameter parsing only.
    ///
    /// # Errors
    ///
    /// Returns [`ContentHeaderParseError`] with offsets relative to the raw value.
    pub fn parse_content_type_with_options(
        &self,
        options: &ParameterParseOptions,
    ) -> Result<ContentType, ContentHeaderParseError> {
        ContentType::parse_with_options(self.raw_value(), options)
    }

    /// Parses the raw value as Content-Disposition with default parameter options.
    ///
    /// The field name is not checked. Parameter diagnostics and syntax offsets
    /// refer to [`Self::raw_value`]. No preliminary text decoding, unfolding,
    /// default attachment fallback, or mutation is performed.
    ///
    /// # Errors
    ///
    /// Returns [`ContentHeaderParseError`] for malformed or empty value syntax.
    pub fn parse_content_disposition(&self) -> Result<ContentDisposition, ContentHeaderParseError> {
        self.parse_content_disposition_with_options(&ParameterParseOptions::default())
    }

    /// Parses raw Content-Disposition syntax with explicit parameter options.
    ///
    /// See [`Self::parse_content_disposition`].
    ///
    /// # Errors
    ///
    /// Returns [`ContentHeaderParseError`] with offsets relative to the raw value.
    pub fn parse_content_disposition_with_options(
        &self,
        options: &ParameterParseOptions,
    ) -> Result<ContentDisposition, ContentHeaderParseError> {
        ContentDisposition::parse_with_options(self.raw_value(), options)
    }

    /// Decodes the value as RFC 2047 text, then unfolds it for display.
    ///
    /// Matches MimeKit's `Value` getter: trims surrounding Unicode whitespace
    /// and removes all remaining CR/LF after decoding. Internal spaces, tabs,
    /// quotes, NUL, and other controls are retained. This is not sanitization,
    /// structured-value parsing, or text suitable for direct header authoring.
    ///
    /// Charset recovery is reported in the returned issues. Their byte ranges
    /// refer to [`raw_value`](Self::raw_value), not the entire record or the
    /// unfolded text. Malformed records have an empty raw value and consequently
    /// return empty text; consult [`is_invalid`](Self::is_invalid) separately.
    ///
    /// Decoding is uncached and never changes raw bytes or the field identifier.
    pub fn decode_value(&self) -> DecodedHeader {
        self.decode_value_with_options(&DecodeOptions::default())
    }

    /// Decodes and unfolds the value with explicit RFC 2047 options.
    ///
    /// The fallback charset overrides guessing for unlabelled invalid UTF-8 or
    /// unsupported encoded-word labels. Valid UTF-8 and recognized charset labels
    /// retain their existing precedence. See [`rfc2047::decode_text_with_options`]
    /// for recovery details. Options apply only to this call and are not stored.
    pub fn decode_value_with_options(&self, options: &DecodeOptions) -> DecodedHeader {
        let mut decoded = rfc2047::decode_text_with_options(self.raw_value(), options);
        // Decode before unfolding: CR/LF inside encoded words must be removed
        // too. Reuse the decoded string's buffer rather than allocating it twice.
        unfold_in_place(&mut decoded.text);
        decoded
    }
}

impl FromStr for Header {
    type Err = HeaderParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

fn parse_field(input: &[u8]) -> Result<(usize, usize), HeaderParseError> {
    // Find the end of the field name before accepting blanks preceding ':'.
    let name_length = input
        .iter()
        .take_while(|&&byte| (33..=126).contains(&byte) && byte != b':')
        .count();
    let mut colon = name_length;
    // Blanks may separate the field name and ':', but cannot occur inside
    // the name. Keep them in the raw field for lossless reserialization.
    while matches!(input.get(colon), Some(b' ' | b'\t')) {
        colon += 1;
    }
    if input.get(colon) == Some(&b':') {
        Ok((name_length, colon))
    } else {
        Err(HeaderParseError { offset: colon })
    }
}

/// A standalone header lacks a valid field/colon boundary.
///
/// This does not describe value syntax, folding, or RFC 2047 decoding errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderParseError {
    offset: usize,
}

impl HeaderParseError {
    /// Returns the input byte offset where a colon was expected.
    ///
    /// May equal the input length when the field or following blanks reach EOF.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for HeaderParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected ':' after header field at byte {}", self.offset)
    }
}

impl Error for HeaderParseError {}

/// A header record exceeded the parser's configured byte limit.
///
/// The parser remains failed until reset. Records emitted before the error
/// remain in the caller's output, but the failing input must not be retried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderTooLong {
    limit: usize,
}

impl HeaderTooLong {
    /// Returns the maximum permitted size of one complete header record.
    pub fn limit(self) -> usize {
        self.limit
    }
}

impl fmt::Display for HeaderTooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "header exceeds the maximum length of {} bytes",
            self.limit
        )
    }
}

impl Error for HeaderTooLong {}

/// An incremental, I/O-independent parser for a single raw header block.
///
/// [`parse`](Self::parse) appends records to a caller-owned vector and returns
/// the number of input bytes consumed. It stops immediately after the blank
/// line separating headers from the body; the remaining input is untouched.
/// LF and CRLF are accepted, including CRLF split between input chunks.
/// Lines beginning with space or tab continue the preceding record, including
/// whitespace-only continuation lines. Bare CR inside a line is preserved.
///
/// At source EOF, call [`finish`](Self::finish) to emit the last record without
/// adding a newline. A lone CR at the start of the final line is treated as a
/// body separator, matching MimeKit. Empty input chunks are not EOF.
///
/// Malformed records are exposed through [`Header::is_invalid`], not silently
/// discarded. Unlike MimeKit's top-level message parser, this low-level scanner
/// also returns an invalid first record. It does not recognize mbox markers,
/// multipart boundaries, structured field values, or encoded words.
///
/// The default maximum record size is 16 KiB, rather than MimeKit's 16 MiB.
/// The limit includes the field, colon, value, continuations, and their newline bytes;
/// it excludes the body separator. Callers must separately bound the number
/// of records and total input if needed. No I/O or finalization occurs on drop.
///
/// ```
/// use mimecraft::HeaderParser;
///
/// let mut parser = HeaderParser::new();
/// let mut headers = Vec::new();
/// let first = b"Subject: hello\r\n\tworld\r";
/// assert_eq!(parser.parse(first, &mut headers)?, first.len());
/// let last = b"\n\r\nbody";
/// let consumed = parser.parse(last, &mut headers)?;
/// assert_eq!(&last[consumed..], b"body");
/// assert!(parser.is_finished());
/// assert!(headers[0].matches_field("subject"));
/// assert_eq!(headers[0].raw_value(), b" hello\r\n\tworld\r\n");
/// assert_eq!(parser.body_separator(), Some(&b"\r\n"[..]));
/// # Ok::<(), mimecraft::HeaderTooLong>(())
/// ```
#[derive(Clone, Debug)]
pub struct HeaderParser {
    pending: Vec<u8>,
    offset: u64,
    track_offsets: bool,
    line_start: bool,
    pending_cr: bool,
    finished: bool,
    separator: Option<&'static [u8]>,
    max_header_length: usize,
    error: Option<HeaderTooLong>,
}

impl HeaderParser {
    /// Creates a parser with a maximum header record size of 16 KiB.
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            offset: 0,
            track_offsets: false,
            line_start: true,
            pending_cr: false,
            finished: false,
            separator: None,
            max_header_length: 16 * 1024,
            error: None,
        }
    }

    /// Creates a parser with a positive maximum size for each header record.
    pub fn with_max_header_length(limit: NonZeroUsize) -> Self {
        Self {
            max_header_length: limit.get(),
            ..Self::new()
        }
    }

    pub(crate) fn set_stream_offset(&mut self, offset: u64) {
        self.offset = offset;
        self.track_offsets = true;
    }

    /// Returns the configured maximum size of a header record in bytes.
    pub fn max_header_length(&self) -> usize {
        self.max_header_length
    }

    /// Returns whether a body separator or explicit end of input was reached.
    ///
    /// An error does not mark the parser finished; it remains failed until reset.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Returns the exact body separator: LF, CRLF, or a lone CR at EOF.
    ///
    /// Returns `None` if no separator has been seen, including after finishing
    /// truncated headers. The separator is not included in any header record.
    pub fn body_separator(&self) -> Option<&[u8]> {
        self.separator
    }

    /// Reads a header block from a buffered source, appending completed records.
    ///
    /// Stops after the body separator, leaving body bytes available through the
    /// same reader. The reader may prefetch; do not discard its buffer or read
    /// directly from its underlying source afterward. No seeking is required.
    /// EOF calls [`finish`](Self::finish), preserving unterminated records.
    /// An already-finished parser performs no I/O. Existing parser state and
    /// output are retained, allowing continuation after earlier buffer parsing.
    /// Emitted headers retain byte offsets from the start of this parser's
    /// input, including any preceding bytes supplied to [`parse`](Self::parse).
    /// No underlying seek position is queried. After reset, offsets restart at
    /// zero even when the same reader is used.
    ///
    /// [`body_separator`](Self::body_separator) retains the exact separator.
    /// The record limit still applies, but total bytes and record count are not
    /// limited. Interrupted reads are retried.
    ///
    /// # Errors
    ///
    /// Read errors preserve parser state and previously appended records; retry
    /// with the same reader, parser, and output. An oversized record returns
    /// [`io::ErrorKind::InvalidData`] containing [`HeaderTooLong`]. That error is
    /// terminal: abandon the block rather than retrying. Its reader position is
    /// unspecified, and resetting the parser does not rewind the reader.
    pub fn read_from<R: BufRead + ?Sized>(
        &mut self,
        reader: &mut R,
        output: &mut Vec<Header>,
    ) -> io::Result<()> {
        if let Some(error) = self.error {
            return Err(io::Error::new(io::ErrorKind::InvalidData, error));
        }
        self.track_offsets = true;
        while !self.finished {
            let input = match reader.fill_buf() {
                Ok(input) => input,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if input.is_empty() {
                self.finish(output)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                break;
            }
            // Consume only the scanner's prefix, not the whole read-ahead
            // buffer: the caller must still be able to read the message body.
            let consumed = self
                .parse(input, output)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            reader.consume(consumed);
        }
        Ok(())
    }

    /// Consumes header bytes and appends complete records, preserving order.
    ///
    /// Returns the number of bytes consumed from this slice, including a body
    /// separator if encountered. Save the unconsumed suffix for body processing.
    /// After completion, returns zero without changing the output.
    ///
    /// A record is held until the next line establishes whether it continues,
    /// a separator is seen, or [`finish`](Self::finish) is called. Empty input
    /// consumes nothing and never finalizes.
    /// Buffer-only parsing has no associated stream offsets;
    /// [`read_from`](Self::read_from) enables tracking for subsequently emitted records.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderTooLong`] before buffering bytes beyond the limit.
    /// Earlier records may already have been emitted and part of the input
    /// consumed. On error, abandon the block; all later calls return the same
    /// error until [`reset`](Self::reset). This is not a retryable I/O error.
    pub fn parse(
        &mut self,
        input: &[u8],
        output: &mut Vec<Header>,
    ) -> Result<usize, HeaderTooLong> {
        if let Some(error) = self.error {
            return Err(error);
        }

        let mut consumed = 0;
        while consumed < input.len() && !self.finished {
            if self.pending_cr {
                if input[consumed] == b'\n' {
                    self.pending_cr = false;
                    self.separator = Some(b"\r\n");
                    self.finished = true;
                    consumed += 1;
                    break;
                }
                self.append(b"\r")?;
                self.pending_cr = false;
                self.line_start = false;
            }

            if self.line_start {
                let byte = input[consumed];
                // A newline alone does not end a header: leading whitespace on
                // the following line makes it a continuation of the same value.
                if !matches!(byte, b' ' | b'\t') {
                    self.emit(output);
                    match byte {
                        b'\n' => {
                            self.separator = Some(b"\n");
                            self.finished = true;
                            consumed += 1;
                            break;
                        }
                        b'\r' => {
                            // A CR at line start might be the body separator.
                            // Delay classifying it until LF, another byte, or EOF.
                            self.pending_cr = true;
                            consumed += 1;
                            continue;
                        }
                        _ => {}
                    }
                }
                self.line_start = false;
            }

            let remaining = &input[consumed..];
            let count = remaining
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(remaining.len(), |index| index + 1);
            self.append(&remaining[..count])?;
            self.line_start = remaining[count - 1] == b'\n';
            consumed += count;
        }

        Ok(consumed)
    }

    /// Ends input and emits the last record exactly once, without repairing it.
    ///
    /// Repeated calls after completion do nothing. A pending CR at line start
    /// becomes a body separator; a CR inside a header remains part of its value.
    ///
    /// # Errors
    ///
    /// Returns the saved [`HeaderTooLong`] error if parsing previously failed.
    pub fn finish(&mut self, output: &mut Vec<Header>) -> Result<(), HeaderTooLong> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.finished {
            if self.pending_cr {
                self.separator = Some(b"\r");
                self.pending_cr = false;
            }
            self.emit(output);
            self.finished = true;
        }
        Ok(())
    }

    /// Discards pending input, completion, and errors, preserving the size limit.
    ///
    /// Previously emitted records in the caller's output are not modified.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.offset = 0;
        self.track_offsets = false;
        self.line_start = true;
        self.pending_cr = false;
        self.finished = false;
        self.separator = None;
        self.error = None;
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), HeaderTooLong> {
        if bytes.len() > self.max_header_length - self.pending.len() {
            let error = HeaderTooLong {
                limit: self.max_header_length,
            };
            self.error = Some(error);
            return Err(error);
        }
        self.pending.extend_from_slice(bytes);
        Ok(())
    }

    fn emit(&mut self, output: &mut Vec<Header>) {
        if !self.pending.is_empty() {
            // This is the byte-preserving stream path, not MimeKit's standalone
            // strict Header.Parse path; do not synthesize a terminal newline.
            let mut header = Header::from_raw(std::mem::take(&mut self.pending));
            if self.track_offsets {
                header.offset = Some(self.offset);
            }
            self.offset += header.raw.len() as u64;
            output.push(header);
        }
    }
}

impl Default for HeaderParser {
    fn default() -> Self {
        Self::new()
    }
}

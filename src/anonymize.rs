// Adapted from MimeKit's MimeAnonymizer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Byte-level masking of MIME headers for inspecting syntax and layout.
//!
//! [`header`] masks a complete stored record using MimeKit's default field rules.
//! [`header_with_options`] can preserve additional fields using [`HeaderOptions`].
//! The value helpers let callers select a grammar explicitly. Record and value
//! helpers return a new buffer of exactly the input length without decoding,
//! validating, repairing, folding, or mutating headers. Each call starts fresh
//! state; do not split one value across calls. [`write_headers`] and
//! [`write_headers_with_options`] mask records and normalize output newlines.
//!
//! This is not guaranteed removal of sensitive information. Lengths, selected
//! syntax, and potential encoded-word charset/encoding labels remain visible;
//! address masking also preserves escaped bytes. Even malformed labels can pass
//! through verbatim. Content-header masking preserves the leading type or
//! disposition, parameter names, and selected parameter values, even if malformed.
//! Received masking preserves an entire suffix if the date parser accepts it,
//! including any unrelated text ignored by that parser.
//! Complete-record masking preserves valid field names and copies Date,
//! MIME-Version, and Content-Transfer-Encoding records verbatim, even when their
//! values contain private or malformed data. Configured preserved fields also
//! pass through in their entirety before any writer newline conversion.
//! Output may contain non-ASCII bytes, controls, or invalid UTF-8 and is not
//! necessarily valid MIME or safe display text.
//!
//! Pass a [`Header`] to [`header`], or [`Header::raw_value`] to the value helpers.
//! Writers support CRLF/LF conversion, not value reformatting or full-message
//! anonymization.
//! Review output before sharing it. Use [`crate::filters::AnonymizeFilter`] for
//! simpler byte masking that preserves only SPACE, TAB, CR, and LF.

use std::io::{self, Write};

use crate::{
    DateTimeOffset, Header, HeaderId,
    encodings::NewLineFormat,
    filters::{AnonymizeFilter, Dos2UnixFilter, MimeFilter, Unix2DosFilter},
    io::FilteredWriter,
    parse_utils::is_whitespace,
};

const ADDRESS_SPECIALS: &[u8] = b" \t\r\n()<>[]:;@,.";
const RECEIVED_SPECIALS: &[u8] = b"<>[]:@,.";
const WHITESPACE: &[u8] = b" \t\r\n";

/// Options for masking complete header records.
///
/// Defaults to no additional preserved fields; [`header`] and
/// [`header_with_options`] then produce identical output. The three built-in
/// pass-through fields (Date, MIME-Version, Content-Transfer-Encoding) remain
/// preserved regardless of these options.
///
/// Unlike MimeKit's mutable case-insensitive set, this is an owned list using
/// the repository's ASCII header-name matching. It retains supplied spelling,
/// ordering, and duplicates, but order and duplicates do not affect masking.
/// Matching scans the list without allocating a normalized field name.
///
/// These options affect [`header_with_options`] and [`write_headers_with_options`],
/// not the raw-value helpers. They do not traverse a message, MIME tree, or
/// structured report body. Writers still normalize preserved records' newlines.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeaderOptions {
    /// Additional field names whose complete raw records are copied verbatim.
    ///
    /// Uses [`Header::matches_field`]: matches the complete name ignoring ASCII
    /// case, without trimming, Unicode case folding, wildcards, or prefix matching.
    /// Entries are stored as supplied rather than validated or canonicalized;
    /// names with colons, surrounding whitespace, or non-ASCII bytes do not
    /// match valid fields. `""` matches a syntactically valid empty field name.
    /// Malformed records never match and are always byte-masked.
    ///
    /// Preservation overrides every field-specific value helper, without
    /// inspecting or validating the value. This can expose private data,
    /// controls, invalid UTF-8, or embedded header-like lines. Review output
    /// before sharing it.
    pub preserve_headers: Vec<String>,
}

/// Masks a complete stored header using MimeKit's default field-specific rules.
///
/// Preserves the field's spelling, blanks before its colon, and the colon.
/// Chooses a value helper using [`Header::id`], so field matching is
/// ASCII-case-insensitive and does not validate value syntax.
///
/// | Fields | Value handling |
/// | --- | --- |
/// | From, Sender, To, Cc, Bcc, Reply-To, their Resent counterparts, Disposition-Notification-To | [`address_header_value`] |
/// | Content-Id, Message-Id, Original-Message-Id, Resent-Message-Id, References, In-Reply-To | [`address_header_value`] |
/// | Received | [`received_header_value`] |
/// | Content-Type | [`content_type_value`] |
/// | Content-Disposition | [`content_disposition_value`] |
/// | Date, MIME-Version, Content-Transfer-Encoding | Copied verbatim, even invalid values |
/// | All other fields, including unknown and empty names | [`unstructured_header_value`] |
///
/// Unlisted address/date-like fields, including Return-Path and Resent-Date,
/// use unstructured masking, not inferred grammar. Authentication and signature
/// fields also use unstructured masking; no verification is performed.
///
/// If [`Header::is_invalid`] is true, masks the **entire record**, including
/// colons and apparent field text, with [`AnonymizeFilter`]. Only SPACE, TAB, CR,
/// and LF survive. A syntactically valid empty field name is not an invalid
/// record: its colon is retained and its value uses unstructured masking.
///
/// Returns exactly as many bytes as [`Header::raw`], leaving the header unchanged.
/// Unlike MimeKit's formatted stream writer, adds no newline or body separator
/// and performs no newline conversion. Individual value rules can still mask
/// escaped newlines in Received comments. This is a byte buffer, not a reparsed
/// header. Use [`header_with_options`] to preserve additional fields.
/// Full-message anonymization remains separate. See the
/// [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::{Header, anonymize};
///
/// let record = Header::parse(b"tO \t: Alice <alice@example.com>\n")?;
/// assert_eq!(
///     anonymize::header(&record),
///     b"tO \t: xxxxx <xxxxx@xxxxxxx.xxx>\n",
/// );
/// # Ok::<(), mimecraft::HeaderParseError>(())
/// ```
pub fn header(record: &Header) -> Vec<u8> {
    header_with_options(record, &HeaderOptions::default())
}

/// Masks a complete stored header while preserving explicitly selected fields.
///
/// Uses [`header`]'s field dispatch, malformed-record handling, and raw-byte
/// output rules. For a valid record, a match in [`HeaderOptions::preserve_headers`]
/// copies the entire stored record before applying any value helper. Values are
/// not validated; field spelling, whitespace, and even malformed value bytes
/// pass through unchanged. Invalid field/colon syntax is still byte-masked.
///
/// Neither input nor options are mutated. Options are checked afresh on each
/// call, so edits take effect immediately without caching. Every supplied record
/// is handled independently, including duplicate fields and records from
/// caller-parsed status groups. No automatic header-list or body traversal occurs.
///
/// Returns exactly as many bytes as [`Header::raw`], without newline conversion
/// or an added separator. Preserved data may be sensitive; see the
/// [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::{Header, anonymize::{self, HeaderOptions}};
///
/// let record = Header::parse(b"Subject: keep this exact value\n")?;
/// let options = HeaderOptions {
///     preserve_headers: vec!["sUbJeCt".into()],
/// };
/// assert_eq!(anonymize::header_with_options(&record, &options), record.raw());
/// assert_eq!(anonymize::header(&record), b"Subject: xxxx xxxx xxxxx xxxxx\n");
/// # Ok::<(), mimecraft::HeaderParseError>(())
/// ```
pub fn header_with_options(record: &Header, options: &HeaderOptions) -> Vec<u8> {
    if record.is_invalid() {
        // Invalid records have no usable field/value boundary: mask raw bytes
        // instead of dispatching on an apparent field or dropping the record.
        let mut output = Vec::with_capacity(record.raw().len());
        AnonymizeFilter::new().filter(record.raw(), &mut output);
        return output;
    }

    if options
        .preserve_headers
        .iter()
        .any(|field| record.matches_field(field))
    {
        // Don't mask a configured field: preservation takes precedence over
        // the default rules, but never over malformed-record byte masking.
        return record.raw().to_vec();
    }

    let raw_value = record.raw_value();
    let value = match record.id() {
        HeaderId::DispositionNotificationTo
        | HeaderId::ResentReplyTo
        | HeaderId::ResentSender
        | HeaderId::ResentFrom
        | HeaderId::ResentBcc
        | HeaderId::ResentCc
        | HeaderId::ResentTo
        | HeaderId::ReplyTo
        | HeaderId::Sender
        | HeaderId::From
        | HeaderId::Bcc
        | HeaderId::Cc
        | HeaderId::To => address_header_value(raw_value),
        HeaderId::Received => received_header_value(raw_value),
        HeaderId::OriginalMessageId
        | HeaderId::ResentMessageId
        | HeaderId::References
        | HeaderId::InReplyTo
        | HeaderId::MessageId
        | HeaderId::ContentId => {
            // Treat these identifiers like address headers for now, as MimeKit does.
            address_header_value(raw_value)
        }
        HeaderId::ContentDisposition => content_disposition_value(raw_value),
        HeaderId::ContentType => content_type_value(raw_value),
        HeaderId::ArcAuthenticationResults
        | HeaderId::AuthenticationResults
        | HeaderId::ArcMessageSignature
        | HeaderId::ArcSeal
        | HeaderId::DkimSignature => {
            // TODO: consider dedicated authentication/signature masking.
            unstructured_header_value(raw_value)
        }
        HeaderId::ContentTransferEncoding | HeaderId::MimeVersion | HeaderId::Date => {
            // Don't mask these headers, even if their values are malformed.
            return record.raw().to_vec();
        }
        _ => unstructured_header_value(raw_value),
    };

    let mut output = Vec::with_capacity(record.raw().len());
    output.extend_from_slice(record.raw_field());
    output.push(b':');
    output.extend_from_slice(&value);
    output
}

/// Writes masked headers with default preservation rules and CRLF newlines.
///
/// Accepts a borrowed [`crate::HeaderList`], slice, or iterator of records.
/// Retains iteration order and duplicates, without adding a body separator.
/// See [`write_headers_with_options`] for formatting, buffering, and errors.
///
/// # Errors
///
/// Propagates destination write and flush errors, possibly after partial output.
pub fn write_headers<'a, W: Write + ?Sized>(
    headers: impl IntoIterator<Item = &'a Header>,
    output: &mut W,
) -> io::Result<()> {
    write_headers_with_options(
        headers,
        output,
        &HeaderOptions::default(),
        NewLineFormat::Dos,
    )
}

/// Writes masked headers with explicit preservation and output newline settings.
///
/// Applies [`header_with_options`] independently to each complete record, then
/// streams the result through the selected CRLF/LF converter. Even preserved
/// fields have their newlines converted. No values are decoded, re-encoded,
/// refolded, or validated. Headers and options remain unchanged.
///
/// Like MimeKit's anonymizer, malformed records are byte-masked and newline
/// converted too. Unlike [`crate::HeaderList::write_to`], there is no exception
/// for carriage-return-only invalid lines: repeated LF conversion can eventually
/// turn such a line into a blank separator. Output is for inspection, not a
/// guaranteed structure-preserving round trip or removal of sensitive data.
///
/// Missing record terminators and a header/body separator are not added. Bare
/// CR bytes survive unless followed by LF. Newline state spans chunks and
/// adjacent records as one ordered byte stream, including malformed records;
/// masking state starts fresh per record. An unterminated record can therefore
/// run into the next record. Only supplied headers are visited, not MIME bodies.
///
/// Buffers one masked record at a time, not the entire collection. On success,
/// finalizes the newline filter and flushes the borrowed destination, even for
/// empty input; it does not close the destination. If the destination is itself
/// a filter writer, the caller must still finalize that outer destination.
///
/// # Errors
///
/// [`NewLineFormat::Mixed`] returns [`io::ErrorKind::InvalidInput`] before
/// obtaining an iterator or performing any I/O. Write and flush errors propagate
/// without rollback, possibly after partial or complete output. Interrupted
/// writes retry; zero writes fail with [`io::ErrorKind::WriteZero`].
/// Retrying with a fresh iterator starts over, not at the failed output byte.
///
/// ```
/// use mimecraft::{
///     HeaderList,
///     anonymize::{self, HeaderOptions},
///     encodings::NewLineFormat,
/// };
///
/// let headers = HeaderList::load(&mut &b"Subject: private\r\nTo: Alice <a@b.c>\n"[..])?;
/// let options = HeaderOptions { preserve_headers: vec!["subject".into()] };
/// let mut output = Vec::new();
/// anonymize::write_headers_with_options(&headers, &mut output, &options, NewLineFormat::Unix)?;
/// assert_eq!(output, b"Subject: private\nTo: xxxxx <x@x.x>\n");
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn write_headers_with_options<'a, W: Write + ?Sized>(
    headers: impl IntoIterator<Item = &'a Header>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
) -> io::Result<()> {
    match format {
        NewLineFormat::Dos => {
            write_filtered_headers(headers, output, options, Unix2DosFilter::new())
        }
        NewLineFormat::Unix => {
            write_filtered_headers(headers, output, options, Dos2UnixFilter::new())
        }
        NewLineFormat::Mixed => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "header new-line format must be Dos or Unix",
        )),
    }
}

fn write_filtered_headers<'a, W: Write + ?Sized, F: MimeFilter>(
    headers: impl IntoIterator<Item = &'a Header>,
    output: &mut W,
    options: &HeaderOptions,
    filter: F,
) -> io::Result<()> {
    let mut filtered = FilteredWriter::new(output, filter);
    for record in headers {
        // Mask whole values before chunked I/O: keywords, dates, and encoded
        // words can depend on bytes beyond a writer's buffer boundary.
        filtered.write_all(&header_with_options(record, options))?;
    }
    // MimeKit's filtered Flush finalizes filters; Rust's Write::flush does not.
    // Finish explicitly so a trailing CR held by Dos2Unix is not lost.
    filtered.finish()
}

/// Masks an unstructured raw header value, retaining potential encoded-word syntax.
///
/// Ordinary bytes become `x`, except SPACE, TAB, CR, LF, `=`, and `?`.
/// A permissive state machine preserves potential charset and encoding labels,
/// masks payload bytes except whitespace and `?`, and retains a closing `?=`.
/// No label validation or encoded-word decoding occurs. Malformed and incomplete
/// sequences follow MimeKit's transitions, not the RFC 2047 decoder's grammar.
///
/// Quotation marks and backslashes have no special treatment outside labels.
/// Returns exactly one byte per input byte; see the [module documentation](self)
/// for scope and limitations.
///
/// ```
/// use mimecraft::anonymize::unstructured_header_value;
///
/// assert_eq!(
///     unstructured_header_value(b" =?utf-8?q?hello_world?=\r\n"),
///     b" =?utf-8?q?xxxxxxxxxxx?=\r\n",
/// );
/// ```
pub fn unstructured_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut word = EncodedWordState::None;
    raw_value
        .iter()
        .map(|&byte| word.mask(byte, WHITESPACE))
        .collect()
}

/// Masks an address-like raw value while retaining punctuation, quotes, and escapes.
///
/// Outside quoted strings, preserves SPACE, TAB, CR, LF, `()<>[]:;@,.`, `=`,
/// and `?`, with the same potential encoded-word handling as
/// [`unstructured_header_value`]. This does not parse mailboxes, comments, groups,
/// or domain literals; malformed address syntax is retained rather than repaired.
///
/// Double quotes and backslashes are retained. An odd run of backslashes escapes
/// the following byte, which is copied verbatim, even inside quotes. Unescaped
/// quotes toggle quoted-string handling: only CR/LF survive masking inside
/// quotes, while spaces and tabs become `x`. Encoded-word state is paused, not
/// reset, while processing quotes, escapes, or quoted text.
///
/// Returns exactly one byte per input byte. Escaped data and potential labels
/// may remain readable; see the [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::anonymize::address_header_value;
///
/// assert_eq!(
///     address_header_value(b" Alice <alice@example.com>\r\n"),
///     b" xxxxx <xxxxx@xxxxxxx.xxx>\r\n",
/// );
/// ```
pub fn address_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(raw_value.len());
    let mut word = EncodedWordState::None;
    let mut escaped = false;
    let mut quoted = false;

    for &byte in raw_value {
        let masked = match byte {
            b'\\' => {
                escaped = !escaped;
                byte
            }
            b'"' => {
                if escaped {
                    escaped = false;
                } else {
                    quoted = !quoted;
                }
                byte
            }
            _ if escaped => {
                escaped = false;
                byte
            }
            _ if quoted => {
                // Inside quoted strings, preserve CR/LF for inspecting folding,
                // but mask spaces and tabs along with the other quoted text.
                if matches!(byte, b'\r' | b'\n') {
                    byte
                } else {
                    b'x'
                }
            }
            _ => word.mask(byte, ADDRESS_SPECIALS),
        };
        output.push(masked);
    }
    output
}

/// Masks a raw Received value, retaining trace syntax and recognized date suffixes.
///
/// Preserves SPACE, TAB, CR, LF, `<>[]:@,.`, and semicolons outside comments.
/// The keywords `from`, `by`, `via`, `with`, `id`, and `for` retain their original
/// spelling when matched ASCII-case-insensitively at a token boundary and
/// followed by MIME whitespace. An exact keyword at EOF is masked. Comments
/// also delimit tokens; punctuation such as `<` does not start a new token.
///
/// Nested comments are scanned iteratively. Parentheses, backslashes, the same
/// punctuation, and whitespace survive inside comments, but keywords do not.
/// An odd backslash run masks the following byte, including whitespace or a
/// parenthesis, instead of interpreting it. Unclosed comments remain comments
/// through EOF; unmatched closing parentheses outside comments are masked.
///
/// After each semicolon outside comments, skips MIME whitespace and tries
/// [`DateTimeOffset::parse`] on the remaining slice. If accepted, the **entire
/// suffix** is copied verbatim, including comments, ignored tokens, and trailing
/// data. Otherwise masking resumes. This is tolerant date recognition, not
/// validation or formatting; missing times and other recovered forms can pass.
///
/// Returns a new buffer of exactly the input length. Each call starts fresh;
/// lookahead means masking a prefix can differ from that prefix of the full
/// result. No encoded-word or charset decoding occurs. Preserved dates and
/// unrelated suffix data may be sensitive; see the [module documentation](self).
///
/// ```
/// use mimecraft::anonymize::received_header_value;
///
/// assert_eq!(
///     received_header_value(b" from mail.example; 15 Nov 2017 14:16:18 -0000\r\n"),
///     b" from xxxx.xxxxxxx; 15 Nov 2017 14:16:18 -0000\r\n",
/// );
/// ```
pub fn received_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = raw_value.to_vec();
    let mut index = 0;

    while index < raw_value.len() {
        if is_whitespace(raw_value[index]) {
            index += 1;
        } else if raw_value[index] == b'(' {
            // Consume '(' and scan the comment, including nested comments.
            let mut depth = 1usize;
            let mut escaped = false;
            index += 1;

            while index < raw_value.len() && depth > 0 {
                let byte = raw_value[index];
                match byte {
                    b'\\' => escaped = !escaped,
                    _ if escaped => {
                        // Escaped bytes are masked even when they are whitespace
                        // or would otherwise open or close a comment.
                        output[index] = b'x';
                        escaped = false;
                    }
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ if !is_whitespace(byte) && !RECEIVED_SPECIALS.contains(&byte) => {
                        // Mask everything else in the comment.
                        output[index] = b'x';
                    }
                    _ => {}
                }
                index += 1;
            }
        } else if raw_value[index] == b';' {
            index += 1;
            while index < raw_value.len() && is_whitespace(raw_value[index]) {
                index += 1;
            }

            // This might be a date. Like MimeKit, don't mask any of the suffix
            // if the date parser accepts it, even bytes the parser ignores.
            if DateTimeOffset::parse(&raw_value[index..]).is_ok() {
                break;
            }
        } else if let Some(length) = received_keyword_length(&raw_value[index..]) {
            index += length;
        } else {
            while index < raw_value.len() {
                let byte = raw_value[index];
                if is_whitespace(byte) || matches!(byte, b';' | b'(') {
                    break;
                }
                if !RECEIVED_SPECIALS.contains(&byte) {
                    output[index] = b'x';
                }
                index += 1;
            }
        }
    }
    output
}

fn received_keyword_length(input: &[u8]) -> Option<usize> {
    // Look for: from, by, via, with, id, for. All are at most four bytes,
    // and even an exact match must have a following MIME whitespace byte.
    let length = input.iter().take(5).position(|&byte| is_whitespace(byte))?;
    let word = &input[..length];
    if word.eq_ignore_ascii_case(b"from")
        || word.eq_ignore_ascii_case(b"by")
        || word.eq_ignore_ascii_case(b"via")
        || word.eq_ignore_ascii_case(b"with")
        || word.eq_ignore_ascii_case(b"id")
        || word.eq_ignore_ascii_case(b"for")
    {
        Some(length)
    } else {
        None
    }
}

/// Masks parameter values in a raw Content-Type value, preserving MIME structure.
///
/// Copies everything through the first semicolon verbatim, without validating
/// the media type. Parameter names and continuation suffixes also pass through.
/// Values named `boundary`, `charset`, `delsp`, or `format` are preserved using
/// ASCII-case-insensitive comparison of the name before its first `*`.
/// Names are not trimmed or decoded, and continuation suffixes are not validated.
///
/// Other value bytes become `x`. Outside quotes, SPACE, TAB, CR, LF, and
/// separating semicolons survive. Inside quoted values, quotes, backslashes,
/// and CR/LF survive; all other bytes, including spaces, tabs, and escaped text,
/// are masked. No RFC 2231 or encoded-word decoding occurs.
///
/// Follows MimeKit's permissive parameter states, not [`crate::ParameterList`]
/// parsing. In particular, an incomplete starred parameter followed by `;`
/// retains its accumulated base name when reading the next parameter. This can
/// change which subsequent values are preserved. An unclosed quote keeps later
/// semicolons inside that value. Missing semicolons do not start new parameters.
///
/// Returns a new buffer of exactly the input length, with fresh state per call.
/// Leading text, names, and preserved values may contain sensitive data or
/// malformed bytes; see the [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::anonymize::content_type_value;
///
/// assert_eq!(
///     content_type_value(b" text/plain; charset=utf-8; name=report.txt\r\n"),
///     b" text/plain; charset=utf-8; name=xxxxxxxxxx\r\n",
/// );
/// ```
pub fn content_type_value(raw_value: &[u8]) -> Vec<u8> {
    // Don't mask the MIME type; only parameter values are candidates.
    content_header_value(raw_value)
}

/// Masks parameter values in a raw Content-Disposition value.
///
/// Preserves everything through the first semicolon, including an unrecognized
/// disposition token. Uses exactly the same parameter rules as
/// [`content_type_value`], including preservation of `boundary`, `charset`,
/// `delsp`, and `format` values even on Content-Disposition. Other values,
/// including filenames, sizes, and dates, are masked rather than decoded.
///
/// Returns a new buffer of exactly the input length, with fresh state per call.
/// This does not validate syntax or guarantee removal of sensitive information;
/// see the [module documentation](self) for limitations.
///
/// ```
/// use mimecraft::anonymize::content_disposition_value;
///
/// assert_eq!(
///     content_disposition_value(b" attachment; filename=\"my file.txt\"\r\n"),
///     b" attachment; filename=\"xxxxxxxxxxx\"\r\n",
/// );
/// ```
pub fn content_disposition_value(raw_value: &[u8]) -> Vec<u8> {
    // Don't mask the "attachment" or "inline" part.
    content_header_value(raw_value)
}

#[derive(Clone, Copy)]
enum ParameterState {
    Semicolon,
    Name,
    NameStar,
    Value,
}

fn is_preserved_parameter_name(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"boundary")
        || name.eq_ignore_ascii_case(b"charset")
        || name.eq_ignore_ascii_case(b"delsp")
        || name.eq_ignore_ascii_case(b"format")
}

fn content_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = raw_value.to_vec();
    let Some(start) = raw_value.iter().position(|&byte| byte == b';') else {
        return output;
    };
    let mut state = ParameterState::Semicolon;
    let mut name = Vec::with_capacity(16);
    let mut escaped = false;
    let mut quoted = false;
    let mut preserve = false;

    for (&byte, masked) in raw_value[start + 1..].iter().zip(&mut output[start + 1..]) {
        if matches!(state, ParameterState::Semicolon) && byte != b';' && !is_whitespace(byte) {
            state = ParameterState::Name;
        }
        match state {
            ParameterState::Semicolon => {
                // Preserve repeated semicolons and intervening whitespace.
            }
            ParameterState::Name => match byte {
                b'=' => {
                    preserve = is_preserved_parameter_name(&name);
                    state = ParameterState::Value;
                }
                b';' => {
                    // An incomplete parameter name: recover at the semicolon.
                    state = ParameterState::Semicolon;
                    name.clear();
                }
                b'*' => state = ParameterState::NameStar,
                _ => name.push(byte),
            },
            ParameterState::NameStar => match byte {
                b'=' => {
                    preserve = is_preserved_parameter_name(&name);
                    state = ParameterState::Value;
                }
                b';' => {
                    // This starred parameter seems incomplete. Like MimeKit,
                    // resume at the next name without clearing the old base name.
                    state = ParameterState::Semicolon;
                }
                _ => {}
            },
            ParameterState::Value => match byte {
                b'"' => {
                    if escaped {
                        escaped = false;
                    } else {
                        quoted = !quoted;
                    }
                }
                b'\\' if quoted => escaped = !escaped,
                b'\r' | b'\n' if quoted => {
                    // Keep CR/LF for inspecting folding within a quoted value.
                    escaped = false;
                }
                _ if quoted => {
                    if !preserve {
                        *masked = b'x';
                    }
                    escaped = false;
                }
                b';' => {
                    state = ParameterState::Semicolon;
                    name.clear();
                }
                _ if !preserve && !is_whitespace(byte) => *masked = b'x',
                _ => {}
            },
        }
    }
    output
}

#[derive(Clone, Copy)]
enum EncodedWordState {
    None,
    Equals,
    EqualsQuestion,
    Charset,
    CharsetQuestion,
    Encoding,
    EncodingQuestion,
    Payload,
    PayloadQuestion,
}

impl EncodedWordState {
    fn mask(&mut self, byte: u8, specials: &[u8]) -> u8 {
        match byte {
            b'=' => match self {
                Self::None => {
                    *self = Self::Equals;
                    byte
                }
                Self::Payload => {
                    // Mask '=' inside the payload, including encoding padding.
                    b'x'
                }
                _ => {
                    // Close ?= or leave an interrupted encoded-word sequence.
                    *self = Self::None;
                    byte
                }
            },
            b'?' => {
                *self = match self {
                    Self::Equals => Self::EqualsQuestion,
                    Self::Charset => Self::CharsetQuestion,
                    Self::Encoding => Self::EncodingQuestion,
                    Self::Payload => Self::PayloadQuestion,
                    // A normal '?' passes through too; unexpected transitions
                    // leave encoded-word mode rather than rejecting the value.
                    _ => Self::None,
                };
                byte
            }
            _ => match self {
                Self::EqualsQuestion | Self::Charset => {
                    // Let the potential charset name pass through unchanged.
                    *self = Self::Charset;
                    byte
                }
                Self::CharsetQuestion | Self::Encoding => {
                    // Likewise preserve the encoding name without validation.
                    *self = Self::Encoding;
                    byte
                }
                Self::EncodingQuestion | Self::Payload => {
                    *self = Self::Payload;
                    // Whitespace inside a payload can affect MIME tokenization,
                    // especially malformed folding, so keep it visible.
                    if is_whitespace(byte) { byte } else { b'x' }
                }
                Self::None | Self::Equals | Self::PayloadQuestion => {
                    // Reprocess unexpected bytes as ordinary text. Returning
                    // their masked value avoids MimeKit's one-byte cursor rewind.
                    *self = Self::None;
                    if specials.contains(&byte) { byte } else { b'x' }
                }
            },
        }
    }
}

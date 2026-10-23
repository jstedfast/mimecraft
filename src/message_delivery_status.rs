// Adapted from MimeKit's MessageDeliveryStatus.cs and MimeParser.ParseStatusGroup.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Cursor, Read, Seek},
    num::NonZeroUsize,
};

use crate::{
    ContentEncoding, ContentHeaderParseError, Header, HeaderList, MimeContent, MimePart,
    encodings::NewLineFormat, filters::encoder_filter_with_options,
};

/// Resource limits for reading detached delivery-status header groups.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryStatusParseOptions {
    /// Maximum total bytes produced by decoding. Defaults to 64 MiB.
    ///
    /// Counts the outer decoded body plus the separately decoded remainder,
    /// when present. This bounds buffered payload, not total allocation overhead
    /// or encoded input consumed by tolerant transfer decoders.
    pub max_decoded_bytes: usize,
    /// Maximum raw bytes per folded header record. Defaults to 16 KiB.
    pub max_header_length: usize,
}

impl Default for DeliveryStatusParseOptions {
    fn default() -> Self {
        Self {
            max_decoded_bytes: 64 * 1024 * 1024,
            max_header_length: 16 * 1024,
        }
    }
}

/// A delivery-status leaf with explicit, detached header-group access.
///
/// Wraps [`MimePart`] for message/delivery-status or message/global-delivery-status.
/// The first group describes the message; later groups describe recipients.
/// Fields remain raw [`HeaderList`] records: required fields, status codes, and
/// recipient syntax are not validated. No charset conversion occurs.
///
/// Loading a MIME tree leaves these payloads as ordinary leaves. Extract a leaf
/// and use [`Self::from_part`] to obtain this view. Parsing never caches groups
/// or rewrites content. Editing returned groups has no effect until explicitly
/// applying [`Self::set_status_groups`], available only for owned memory bodies.
/// Generic and borrowed streams retain read-only group access.
///
/// ```
/// use mimecraft::{Header, HeaderList, MessageDeliveryStatus};
///
/// let mut status = MessageDeliveryStatus::new();
/// let message = HeaderList::from(vec![Header::parse(b"Reporting-MTA: dns; example.com")?]);
/// let recipient = HeaderList::from(vec![
///     Header::parse(b"Final-Recipient: rfc822; user@example.com")?,
///     Header::parse(b"Action: failed")?,
///     Header::parse(b"Status: 5.1.1")?,
/// ]);
/// status.set_status_groups(&[message, recipient])?;
/// assert_eq!(status.status_groups()?.len(), 2);
/// let mut output = Vec::new();
/// status.part_mut().write_to(&mut output)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MessageDeliveryStatus<R = Cursor<Vec<u8>>> {
    part: MimePart<R>,
}

impl MessageDeliveryStatus {
    /// Creates message/delivery-status with empty, unencoded content.
    pub fn new() -> Self {
        let header = Header::from_formatted("Content-Type: message/delivery-status\r\n".into());
        Self {
            part: MimePart::from_parts(
                vec![header].into(),
                MimeContent::new(Cursor::new(Vec::new())),
            ),
        }
    }

    /// Atomically replaces the body with groups serialized using CRLF.
    ///
    /// See [`Self::set_status_groups_with_new_line_format`] for encoding,
    /// validation, and metadata-preservation rules.
    ///
    /// # Errors
    ///
    /// Invalid metadata or groups leave the old body and its position unchanged.
    pub fn set_status_groups(&mut self, groups: &[HeaderList]) -> Result<(), DeliveryStatusError> {
        self.set_status_groups_with_new_line_format(groups, NewLineFormat::Dos)
    }

    /// Atomically serializes groups with CRLF or LF and replaces owned content.
    ///
    /// Terminates each record and adds a blank separator after each group.
    /// Preserves field order, duplicates, spelling, folds, and value bytes using
    /// the existing header writer. An empty slice clears the content; individual
    /// empty groups are rejected because blank lines cannot preserve them.
    ///
    /// A group's Base64/QP/UU Content-Transfer-Encoding encodes all later groups,
    /// once, matching the read-side compatibility rule. Later encoding fields
    /// within that remainder are ordinary data. UU uses a `status.txt` wrapper.
    /// Unlike copying decoded groups verbatim, this keeps retained encoding
    /// metadata consistent on a subsequent read.
    ///
    /// Outer headers remain byte-for-byte unchanged, including Content-Length
    /// and the requested Content-Transfer-Encoding; callers must update stale
    /// descriptive fields themselves. New stored encoding is `Default`, with
    /// position zero and no newline hint. No old source I/O occurs.
    ///
    /// # Errors
    ///
    /// Missing/invalid/wrong Content-Type, `Mixed`, empty member groups, and
    /// unknown active group encodings fail before replacing any content.
    pub fn set_status_groups_with_new_line_format(
        &mut self,
        groups: &[HeaderList],
        format: NewLineFormat,
    ) -> Result<(), DeliveryStatusError> {
        self.check_type()?;
        let bytes = serialize_groups(groups, format).map_err(DeliveryStatusError::Io)?;
        self.part.set_content(MimeContent::new(Cursor::new(bytes)));
        Ok(())
    }
}

impl Default for MessageDeliveryStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Read + Seek> MessageDeliveryStatus<R> {
    /// Takes a leaf after checking its first Content-Type, without source I/O.
    ///
    /// Accepts delivery-status and global-delivery-status, ignoring ASCII case.
    ///
    /// # Errors
    ///
    /// Missing, invalid, or other media types return errors. Failure drops the
    /// owned part; borrowed streams remain open and at their original position.
    pub fn from_part(part: MimePart<R>) -> Result<Self, DeliveryStatusError> {
        let status = Self { part };
        status.check_type()?;
        Ok(status)
    }

    /// Returns the underlying leaf and its raw metadata/content.
    pub fn part(&self) -> &MimePart<R> {
        &self.part
    }

    /// Edits the underlying leaf; later group operations recheck its media type.
    pub fn part_mut(&mut self) -> &mut MimePart<R> {
        &mut self.part
    }

    /// Extracts the leaf without copying bytes or accessing its source.
    pub fn into_part(self) -> MimePart<R> {
        self.part
    }

    /// Reads detached status groups using default resource limits.
    ///
    /// See [`Self::status_groups_with_options`] for parsing and source behavior.
    ///
    /// # Errors
    ///
    /// Returns metadata, resource-limit, and I/O errors without partial groups.
    pub fn status_groups(&mut self) -> Result<Vec<HeaderList>, DeliveryStatusError> {
        self.status_groups_with_options(&DeliveryStatusParseOptions::default())
    }

    /// Reads transfer-decoded content as blank-line-separated raw header groups.
    ///
    /// Skips leading, repeated, and trailing empty CRLF/LF lines. Accepts a final
    /// unterminated record. Empty content and a lone trailing CR contribute no
    /// group, following the header parser. Malformed records are retained as in
    /// [`HeaderList::load`], not silently dropped or interpreted as body text.
    ///
    /// Office365 sometimes puts a transfer-encoding field in a group and encodes
    /// the remainder separately. The first active Base64/QP/UU field switches
    /// parsing to a decoded remainder. This happens at most once; fields within
    /// that decoded remainder are not recursively interpreted. Missing, 7bit,
    /// 8bit, and binary fields do not switch. Unknown values are errors.
    /// Transfer codecs retain their existing tolerance, including ignoring
    /// malformed Base64 characters; this is not transfer-encoding validation.
    ///
    /// Reads the entire outer decoded body before parsing. Successful decoding
    /// rewinds its source to zero, even if subsequent header parsing fails.
    /// Raw headers, encoded bytes, and encoding labels are unchanged; each call
    /// parses afresh. Decoded groups and any remainder are held in memory.
    ///
    /// # Errors
    ///
    /// Zero limits and invalid media metadata fail before I/O. Limit breaches,
    /// unknown active encodings, oversized headers, and source errors return no
    /// partial result. Failed decoding follows [`MimeContent::decode_to`]'s
    /// source-position rules, without rollback or swallowed errors.
    pub fn status_groups_with_options(
        &mut self,
        options: &DeliveryStatusParseOptions,
    ) -> Result<Vec<HeaderList>, DeliveryStatusError> {
        let header_limit = NonZeroUsize::new(options.max_header_length)
            .filter(|_| options.max_decoded_bytes > 0)
            .ok_or_else(|| {
                DeliveryStatusError::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "delivery-status limits must be positive",
                ))
            })?;
        self.check_type()?;
        // Finish outer decoding and rewind before parsing, so a later malformed
        // group does not leave an otherwise successfully read source at EOF.
        let bytes = self
            .part
            .content_mut()
            .decode_to_vec_with_limit(options.max_decoded_bytes)
            .map_err(DeliveryStatusError::Io)?;
        let mut input = bytes.as_slice();
        let mut groups = Vec::new();
        // RFC 3464 status groups are blocks of field/value pairs separated by
        // blank lines: per-message fields first, then per-recipient fields.
        //
        // Office365 sometimes adds Content-Transfer-Encoding to the first group
        // and base64-encodes everything after it (MimeKit issue #250). Stop at
        // any group declaring an encoding that needs decoding, leaving input
        // positioned after that group's separator for the second parsing pass.
        if let Some(encoding) = read_groups(&mut input, header_limit, &mut groups, true)
            .map_err(DeliveryStatusError::Io)?
        {
            let mut content = MimeContent::with_encoding(Cursor::new(input), encoding);
            // Both decoding stages share one budget; the outer bytes stay buffered.
            let remainder = content
                .decode_to_vec_with_limit(options.max_decoded_bytes - bytes.len())
                .map_err(DeliveryStatusError::Io)?;
            // MimeKit switches decoding only once. Encoding fields inside this
            // decoded remainder are data, not instructions for another pass.
            read_groups(&mut remainder.as_slice(), header_limit, &mut groups, false)
                .map_err(DeliveryStatusError::Io)?;
        }
        Ok(groups)
    }

    fn check_type(&self) -> Result<(), DeliveryStatusError> {
        let content_type = self
            .part
            .content_type()
            .map_err(DeliveryStatusError::ContentType)?
            .ok_or(DeliveryStatusError::MissingContentType)?;
        if content_type.is_mime_type("message", "delivery-status")
            || content_type.is_mime_type("message", "global-delivery-status")
        {
            Ok(())
        } else {
            Err(DeliveryStatusError::NotDeliveryStatus(
                content_type.mime_type(),
            ))
        }
    }
}

fn encoded_remainder(group: &HeaderList) -> io::Result<Option<ContentEncoding>> {
    let encoding = crate::mime_part::transfer_encoding(group)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // 7bit, 8bit, and binary need no decoding; keep parsing subsequent groups
    // normally rather than consuming the one permitted decoding switch.
    Ok(matches!(
        encoding,
        ContentEncoding::Base64 | ContentEncoding::QuotedPrintable | ContentEncoding::UuEncode
    )
    .then_some(encoding))
}

fn read_groups(
    input: &mut &[u8],
    limit: NonZeroUsize,
    groups: &mut Vec<HeaderList>,
    detect_encoding: bool,
) -> io::Result<Option<ContentEncoding>> {
    while !input.is_empty() {
        let group = HeaderList::load_with_max_header_length(input, limit)?;
        if group.is_empty() {
            // Loading consumed a blank separator (or EOF), not a status group.
            // Extra blank lines must not create empty per-recipient entries.
            continue;
        }
        let encoding = if detect_encoding {
            encoded_remainder(&group)?
        } else {
            None
        };
        groups.push(group);
        if encoding.is_some() {
            return Ok(encoding);
        }
    }
    Ok(None)
}

fn serialize_groups(groups: &[HeaderList], format: NewLineFormat) -> io::Result<Vec<u8>> {
    let newline: &[u8] = match format {
        NewLineFormat::Dos => b"\r\n",
        NewLineFormat::Unix => b"\n",
        NewLineFormat::Mixed => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "group newlines must be Dos or Unix",
            ));
        }
    };
    if groups.iter().any(HeaderList::is_empty) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "each status group must contain at least one header record",
        ));
    }
    let mut bytes = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        write_group(group, &mut bytes, format, newline)?;
        if let Some(encoding) = encoded_remainder(group)? {
            // Retaining this group's encoding field over plain recipient bytes
            // would corrupt the next read. Encode all later groups as one
            // remainder, without interpreting their own encoding fields.
            let mut remainder = Vec::new();
            for group in &groups[index + 1..] {
                write_group(group, &mut remainder, format, newline)?;
            }
            let mut filter = encoder_filter_with_options(encoding, 78, format)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            if encoding == ContentEncoding::UuEncode {
                bytes.extend_from_slice(b"begin 0644 status.txt");
                bytes.extend_from_slice(newline);
            }
            filter.filter(&remainder, &mut bytes);
            filter.finish(&mut bytes);
            if encoding == ContentEncoding::UuEncode {
                bytes.extend_from_slice(b"end");
                bytes.extend_from_slice(newline);
            }
            break;
        }
    }
    Ok(bytes)
}

fn write_group(
    group: &HeaderList,
    output: &mut Vec<u8>,
    format: NewLineFormat,
    newline: &[u8],
) -> io::Result<()> {
    group.write_terminated(output, format)?;
    output.extend_from_slice(newline);
    Ok(())
}

/// A delivery-status part could not be wrapped, read, or edited.
#[derive(Debug)]
pub enum DeliveryStatusError {
    /// The first Content-Type is absent.
    MissingContentType,
    /// Content-Type does not describe a supported delivery-status subtype.
    NotDeliveryStatus(String),
    /// The first Content-Type could not be parsed.
    ContentType(ContentHeaderParseError),
    /// Invalid options/groups, unknown encoding, a resource limit, or I/O failed.
    Io(io::Error),
}

impl fmt::Display for DeliveryStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("delivery-status part has no Content-Type"),
            Self::NotDeliveryStatus(value) => {
                write!(f, "expected a delivery-status media type, got {value}")
            }
            Self::ContentType(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl Error for DeliveryStatusError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingContentType | Self::NotDeliveryStatus(_) => None,
            Self::ContentType(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

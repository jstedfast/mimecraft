// Adapted from MimeKit's AttachmentCollection.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Standalone attachment construction for ordinary `Vec<MimeEntity>` collections.
//!
//! These synchronous helpers copy input into owned memory. They infer media types
//! from filenames or accept explicit types, author filename/disposition metadata,
//! and choose a requested wire encoding without encoding the stored bytes.
//! Only message/rfc822 is parsed into a message wrapper; malformed messages return
//! errors even when the type was inferred. Choose application/octet-stream
//! explicitly to attach such data as an opaque binary file.
//!
//! ```
//! use mimecraft::{BodyBuilder, attachment};
//!
//! let mut builder = BodyBuilder::new();
//! builder.text_body = Some("See the attached report.".into());
//! builder.attachments.push(attachment::from_bytes("report.txt", b"report\n")?);
//! let body = builder.into_message_body()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{self, Cursor, Read},
    path::Path,
};

use crate::{
    BestEncodingError, ContentDisposition, ContentEncoding, ContentHeaderParseError, ContentType,
    EncodingConstraint, Header, HeaderFormatError, HeaderList, MessagePart, MimeContent,
    MimeEntity, MimeEntityParseOptions, MimeMessage, MimePart, MimeTypes,
};

/// Metadata and input limits for standalone attachment construction.
#[derive(Clone, Debug, Default)]
pub struct AttachmentOptions {
    /// Explicit media type, or `None` to infer it using [`MimeTypes`].
    ///
    /// Unknown extensions become application/octet-stream. Ordinary parts retain
    /// supplied parameters except `name`, which is replaced by the filename.
    /// Like MimeKit, parsed messages receive a fresh message/rfc822 wrapper.
    pub content_type: Option<ContentType>,
    /// Use inline disposition and a relative Content-Location instead of attachment.
    ///
    /// Defaults to false. Content-Location percent-encodes the UTF-8 filename;
    /// URI delimiters in a filename are literal, not schemes, queries, or fragments.
    /// Content-Id is not generated. Adding the result to a linked-resource vector
    /// does not implicitly change this option.
    pub linked_resource: bool,
    /// Parsing options for embedded messages and the input budget for all parts.
    ///
    /// `max_input_bytes` bounds raw attachment data, defaulting to 64 MiB.
    /// Other parser limits apply to message/rfc822 contents. All limits must be
    /// positive, including when constructing an opaque part. These budgets do
    /// not bound decoded payloads or total allocation overhead.
    pub parse_options: MimeEntityParseOptions,
}

/// Creates an attachment by copying bytes and inferring the type from its filename.
///
/// See [`from_bytes_with_options`] for filename, metadata, and error behavior.
///
/// # Errors
///
/// Returns filename, formatting, input-limit, encoding-analysis, or message errors.
pub fn from_bytes(file_name: &str, data: &[u8]) -> Result<MimeEntity, AttachmentError> {
    from_bytes_with_options(file_name, data, &AttachmentOptions::default())
}

/// Creates an attachment by copying bytes with explicit metadata and limits.
///
/// Takes the final filename component, recognizing both slash styles on every
/// platform. Empty names, dot components, whitespace-only names, and control
/// characters are rejected. This is MIME metadata, not filesystem validation.
/// Filename parameters use the existing checked Content-Type and disposition
/// formatters; linked locations are ASCII relative URIs with a 998-byte physical
/// header-line limit. Options and metadata are checked before copying content.
///
/// Text uses MimeKit's seven-bit encoding heuristic, with its default 78-column
/// content-line threshold. Other opaque parts request Base64. Bytes are neither
/// charset-decoded nor transfer-encoded; charset parameters are not inferred.
/// Stored content starts at position zero with encoding `Unspecified`.
///
/// Message/rfc822 data is parsed into a [`MessagePart`] without a transfer-encoding
/// header. Both inferred and explicitly requested messages propagate parser
/// errors instead of falling back to a binary part. No partial entity is returned.
///
/// # Errors
///
/// Returns [`AttachmentError`] for invalid options/names, metadata formatting,
/// excess input, encoding analysis, or embedded-message parsing failures.
pub fn from_bytes_with_options(
    file_name: &str,
    data: &[u8],
    options: &AttachmentOptions,
) -> Result<MimeEntity, AttachmentError> {
    let prepared = prepare(file_name, options)?;
    if data.len() > options.parse_options.max_input_bytes {
        return Err(AttachmentError::InputTooLarge {
            limit: options.parse_options.max_input_bytes,
        });
    }
    create(prepared, data.to_vec(), options)
}

/// Copies a reader's remaining bytes into an attachment with an inferred type.
///
/// Does not seek or close the borrowed reader. See [`from_reader_with_options`].
///
/// # Errors
///
/// Returns metadata, input-limit, read, encoding-analysis, or message errors.
pub fn from_reader<R: Read + ?Sized>(
    file_name: &str,
    reader: &mut R,
) -> Result<MimeEntity, AttachmentError> {
    from_reader_with_options(file_name, reader, &AttachmentOptions::default())
}

/// Copies a reader's remaining bytes using explicit metadata and limits.
///
/// Uses [`from_bytes_with_options`]'s construction rules. Nonseekable readers
/// work, and the returned entity does not borrow the input. Metadata failures
/// precede I/O. Successful reading consumes through EOF; an exact-limit input
/// probes EOF, while oversized input consumes at most one byte beyond the limit.
/// Interrupted reads are retried. No cleanup read/seek occurs after failure.
///
/// # Errors
///
/// Propagates input errors without closing or rewinding the caller's reader.
/// Message parsing happens after the bounded copy and may fail after EOF.
pub fn from_reader_with_options<R: Read + ?Sized>(
    file_name: &str,
    reader: &mut R,
    options: &AttachmentOptions,
) -> Result<MimeEntity, AttachmentError> {
    let prepared = prepare(file_name, options)?;
    let data = read_content(reader, options.parse_options.max_input_bytes)?;
    create(prepared, data, options)
}

/// Opens a file read-only and copies it into an attachment with an inferred type.
///
/// See [`from_file_with_options`]. No file handle is retained in the entity.
///
/// # Errors
///
/// Returns metadata, file-open/read, input-limit, encoding-analysis, or message errors.
pub fn from_file<P: AsRef<Path>>(path: P) -> Result<MimeEntity, AttachmentError> {
    from_file_with_options(path, &AttachmentOptions::default())
}

/// Opens a file read-only and copies it with explicit metadata and limits.
///
/// Only the final UTF-8 filename is used in headers, not the directory path.
/// Options and metadata are validated before opening. The handle is closed
/// before return on success or failure. The returned in-memory entity remains
/// usable after the original file is removed. No file timestamps are authored.
///
/// # Errors
///
/// Returns [`AttachmentError::InvalidFileName`] for absent/non-Unicode filenames.
/// Otherwise uses [`from_reader_with_options`]'s errors, plus file-open failures.
pub fn from_file_with_options<P: AsRef<Path>>(
    path: P,
    options: &AttachmentOptions,
) -> Result<MimeEntity, AttachmentError> {
    let path = path.as_ref();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(AttachmentError::InvalidFileName)?;
    let prepared = prepare(file_name, options)?;
    let mut file = File::open(path).map_err(AttachmentError::Io)?;
    let data = read_content(&mut file, options.parse_options.max_input_bytes)?;
    create(prepared, data, options)
}

struct PreparedAttachment {
    headers: HeaderList,
    is_message: bool,
    is_text: bool,
}

fn prepare(
    file_name: &str,
    options: &AttachmentOptions,
) -> Result<PreparedAttachment, AttachmentError> {
    options
        .parse_options
        .validate()
        .map_err(AttachmentError::Options)?;
    let name = file_name.rsplit(['/', '\\']).next().unwrap();
    if name.trim().is_empty() || matches!(name, "." | "..") || name.chars().any(char::is_control) {
        return Err(AttachmentError::InvalidFileName);
    }
    let mut content_type = match &options.content_type {
        Some(value) => value.clone(),
        None => ContentType::parse(MimeTypes::new().mime_type(name).as_bytes())
            .map_err(AttachmentError::ContentType)?,
    };
    let is_message = content_type.is_mime_type("message", "rfc822");
    let is_text = content_type.is_mime_type("text", "*");
    if is_message {
        content_type =
            ContentType::new("message", "rfc822").map_err(AttachmentError::ContentType)?;
    }
    content_type.set_name(Some(name));
    let token = if options.linked_resource {
        ContentDisposition::INLINE
    } else {
        ContentDisposition::ATTACHMENT
    };
    let mut disposition = ContentDisposition::new(token).expect("constant disposition is valid");
    disposition.set_file_name(Some(name));
    let mut headers = HeaderList::new();
    headers.add(
        content_type
            .to_header()
            .map_err(AttachmentError::HeaderFormat)?,
    );
    headers.add(
        disposition
            .to_header()
            .map_err(AttachmentError::HeaderFormat)?,
    );
    if options.linked_resource {
        // Escaping every non-unreserved byte keeps filenames literal, even when
        // they contain URI syntax such as ':', '#', '?', or an existing '%'.
        let location = relative_location(name);
        if location.len() > 997 {
            return Err(AttachmentError::HeaderFormat(
                HeaderFormatError::LineTooLong {
                    required: location.len() + 1,
                    limit: 998,
                },
            ));
        }
        // A URI cannot use RFC 2047 or fold inside its token. Fold before it
        // when needed, retaining the absolute 998-byte physical-line limit.
        let prefix = if "Content-Location: ".len() + location.len() <= 78 {
            "Content-Location: "
        } else {
            "Content-Location:\r\n\t"
        };
        headers.add(Header::from_formatted(format!("{prefix}{location}\r\n")));
    }
    Ok(PreparedAttachment {
        headers,
        is_message,
        is_text,
    })
}

fn relative_location(name: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut location = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            location.push(char::from(byte));
        } else {
            location.push('%');
            location.push(char::from(HEX[(byte >> 4) as usize]));
            location.push(char::from(HEX[(byte & 15) as usize]));
        }
    }
    location
}

fn read_content<R: Read + ?Sized>(
    reader: &mut R,
    limit: usize,
) -> Result<Vec<u8>, AttachmentError> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let remaining = limit - bytes.len();
        let length = remaining.saturating_add(1).min(buffer.len());
        let count = match reader.read(&mut buffer[..length]) {
            Ok(0) => return Ok(bytes),
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(AttachmentError::Io(error)),
        };
        if count > remaining {
            return Err(AttachmentError::InputTooLarge { limit });
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn create(
    prepared: PreparedAttachment,
    data: Vec<u8>,
    options: &AttachmentOptions,
) -> Result<MimeEntity, AttachmentError> {
    if prepared.is_message {
        let message = MimeMessage::load_with_options(&mut data.as_slice(), &options.parse_options)
            .map_err(AttachmentError::Message)?;
        return Ok(MessagePart::from_parts(prepared.headers, Some(message)).into());
    }
    let mut part = MimePart::from_parts(prepared.headers, MimeContent::new(Cursor::new(data)));
    let encoding = if prepared.is_text {
        // TODO: should we try to auto-detect charsets if no charset parameter is specified?
        part.best_encoding(EncodingConstraint::SevenBit)
            .map_err(AttachmentError::BestEncoding)?
    } else {
        ContentEncoding::Base64
    };
    part.set_content_transfer_encoding(encoding);
    Ok(part.into())
}

/// An attachment could not be constructed without returning a partial entity.
#[derive(Debug)]
pub enum AttachmentError {
    /// A filename is missing, non-Unicode, a dot component, or contains controls.
    InvalidFileName,
    /// Resource limits are invalid.
    Options(io::Error),
    /// The inferred media type could not be parsed.
    ContentType(ContentHeaderParseError),
    /// Filename, Content-Type, or Content-Location metadata could not be formatted.
    HeaderFormat(HeaderFormatError),
    /// Raw input exceeds the configured byte budget.
    InputTooLarge {
        /// The allowed input size in bytes.
        limit: usize,
    },
    /// Opening or reading the source failed.
    Io(io::Error),
    /// Embedded message parsing failed, including parser resource limits.
    Message(io::Error),
    /// Text transfer-encoding analysis failed.
    BestEncoding(BestEncodingError),
}

impl fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFileName => f.write_str("attachment filename is invalid"),
            Self::Options(error) => write!(f, "invalid attachment options: {error}"),
            Self::ContentType(error) => error.fmt(f),
            Self::HeaderFormat(error) => error.fmt(f),
            Self::InputTooLarge { limit } => write!(f, "attachment input exceeds {limit} bytes"),
            Self::Io(error) => error.fmt(f),
            Self::Message(error) => write!(f, "attachment message parsing failed: {error}"),
            Self::BestEncoding(error) => error.fmt(f),
        }
    }
}

impl Error for AttachmentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Options(error) | Self::Io(error) | Self::Message(error) => Some(error),
            Self::ContentType(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
            Self::BestEncoding(error) => Some(error),
            Self::InvalidFileName | Self::InputTooLarge { .. } => None,
        }
    }
}

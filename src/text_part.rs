// Adapted from MimeKit's TextPart.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Cursor, Read, Seek},
};

use crate::{
    ContentHeaderParseError, ContentType, HeaderFormatError, MimeContent, MimePart, TextFormat,
    charsets::{Charset, CharsetError},
};

/// A text MIME part with charset-aware access to its body.
///
/// Composes [`MimePart`] rather than inheriting or caching its metadata.
/// [`Self::from_part`] accepts owned or borrowed seekable content. Constructors
/// and text setters create in-memory bodies; setters are available only for
/// `TextPart<Cursor<Vec<u8>>>`, not arbitrary external streams.
///
/// Text operations recheck the first Content-Type after raw edits through
/// [`Self::part_mut`]. Missing, malformed, and non-text types are explicit
/// errors, not silently replaced. Other MIME metadata and serialization remain
/// accessible through the underlying part.
///
/// Unlike MimeKit's automatic text getter, decoding is strict: unsupported
/// charsets and invalid bytes are errors. There is no BOM/HTML charset sniffing,
/// Latin-1 fallback, replacement, newline normalization, or text-format
/// conversion. Explicit BOMs are preserved as characters. Whole-text reads
/// materialize the transfer-decoded bytes and resulting string in memory.
///
/// ```
/// use mimecraft::{ContentEncoding, TextPart};
/// use mimecraft::charsets::Charset;
///
/// let mut text = TextPart::with_charset("plain", "Ren\u{e9}e", Charset::LATIN1)?;
/// text.part_mut().set_content_transfer_encoding(ContentEncoding::Base64);
/// assert_eq!(text.text()?, "Ren\u{e9}e");
/// let mut output = Vec::new();
/// text.part_mut().write_to(&mut output)?;
/// assert!(output.ends_with(b"\r\n\r\nUmVu6WU=\r\n"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Text replacement cannot silently overwrite an arbitrary borrowed stream:
///
/// ```compile_fail
/// use std::io::Cursor;
/// use mimecraft::{ContentType, MimeContent, MimePart, TextPart};
///
/// let mut source = Cursor::new(b"original");
/// let part = MimePart::with_content_type(
///     &ContentType::new("text", "plain").unwrap(),
///     MimeContent::new(&mut source),
/// ).unwrap();
/// let mut text = TextPart::from_part(part).unwrap();
/// text.set_text("replacement").unwrap();
/// ```
#[derive(Debug)]
pub struct TextPart<R = Cursor<Vec<u8>>> {
    part: MimePart<R>,
}

impl TextPart<Cursor<Vec<u8>>> {
    /// Creates an in-memory `text/subtype` part containing UTF-8 text.
    ///
    /// Sets `charset=utf-8` without adding a transfer-encoding header. Input
    /// newlines and any explicit BOM are retained; no BOM is inserted.
    ///
    /// # Errors
    ///
    /// Returns a content-type syntax or header-formatting error.
    pub fn new(subtype: &str, text: &str) -> Result<Self, TextPartError> {
        Self::with_charset(subtype, text, Charset::UTF8)
    }

    /// Creates an in-memory UTF-8 part for a text format.
    ///
    /// Maps Plain/Flowed to `text/plain`, Html to `text/html`, Enriched to
    /// `text/enriched`, and RichText to `text/rtf`. Flowed additionally sets
    /// `format=flowed`. The supplied text is stored literally, without format
    /// conversion, markup validation, flowed wrapping, or space stuffing.
    ///
    /// # Errors
    ///
    /// Returns content-type or header-formatting failures.
    ///
    /// ```
    /// use mimecraft::{TextFormat, TextPart};
    ///
    /// let mut text = TextPart::with_format(TextFormat::Flowed, "already flowed \r\ntext")?;
    /// assert!(text.is_plain()?);
    /// assert!(text.is_flowed()?);
    /// assert_eq!(text.format()?, TextFormat::Flowed);
    /// assert_eq!(text.text()?, "already flowed \r\ntext");
    /// # Ok::<(), mimecraft::TextPartError>(())
    /// ```
    pub fn with_format(format: TextFormat, text: &str) -> Result<Self, TextPartError> {
        Self::with_format_and_charset(format, text, Charset::UTF8)
    }

    /// Creates a part for a text format using an explicit charset.
    ///
    /// Uses [`Self::with_format`]'s media-type mapping and
    /// [`Self::with_charset`]'s strict encoding, without transcoding between
    /// text formats. Flowed construction does not add a `delsp` parameter.
    ///
    /// # Errors
    ///
    /// Returns content-type, header-formatting, or unmappable-text errors.
    pub fn with_format_and_charset(
        format: TextFormat,
        text: &str,
        charset: Charset,
    ) -> Result<Self, TextPartError> {
        let subtype = match format {
            TextFormat::Plain | TextFormat::Flowed => "plain",
            TextFormat::Html => "html",
            TextFormat::Enriched => "enriched",
            TextFormat::RichText => "rtf",
        };
        let mut content_type =
            ContentType::new("text", subtype).map_err(TextPartError::ContentType)?;
        if format == TextFormat::Flowed {
            content_type.set_format(Some("flowed"));
        }
        Self::from_text(content_type, text, charset)
    }

    /// Creates an in-memory text part using a specific charset.
    ///
    /// Stores unencoded body bytes and writes the charset's canonical label.
    /// The caller may separately choose a transfer encoding through the part.
    /// No replacement characters or encoder-generated BOM are inserted.
    ///
    /// # Errors
    ///
    /// Returns a content-type, header-formatting, or unmappable-text error.
    pub fn with_charset(
        subtype: &str,
        text: &str,
        charset: Charset,
    ) -> Result<Self, TextPartError> {
        let content_type = ContentType::new("text", subtype).map_err(TextPartError::ContentType)?;
        Self::from_text(content_type, text, charset)
    }

    fn from_text(
        mut content_type: ContentType,
        text: &str,
        charset: Charset,
    ) -> Result<Self, TextPartError> {
        content_type.set_charset_encoding(Some(charset));
        let bytes = charset
            .encode(text)
            .map_err(TextPartError::Charset)?
            .into_owned();
        let part = MimePart::with_content_type(&content_type, MimeContent::new(Cursor::new(bytes)))
            .map_err(TextPartError::HeaderFormat)?;
        Ok(Self { part })
    }

    /// Replaces text and the charset parameter atomically using UTF-8.
    ///
    /// Like MimeKit's Text setter, this selects UTF-8 even if the old body used
    /// a different charset. See [`Self::set_text_with_charset`] for edit rules.
    ///
    /// # Errors
    ///
    /// Returns metadata or formatting errors without changing the part.
    pub fn set_text(&mut self, text: &str) -> Result<(), TextPartError> {
        self.set_text_with_charset(text, Charset::UTF8)
    }

    /// Atomically replaces the in-memory body and Content-Type charset.
    ///
    /// Encodes and formats before committing either change. Other parameters
    /// survive; Content-Type duplicates collapse at the first slot. Other
    /// headers, including Content-Transfer-Encoding, remain byte-for-byte intact.
    /// The new body has stored encoding `Unspecified`, position zero, and no newline
    /// hint. No old source I/O occurs and the old body is dropped only on success.
    ///
    /// # Errors
    ///
    /// Missing, invalid, or non-text Content-Type, unmappable characters, and
    /// formatting failures leave all headers, old bytes, and position unchanged.
    pub fn set_text_with_charset(
        &mut self,
        text: &str,
        charset: Charset,
    ) -> Result<(), TextPartError> {
        let mut content_type = self.checked_content_type()?;
        let bytes = charset
            .encode(text)
            .map_err(TextPartError::Charset)?
            .into_owned();
        content_type.set_charset_encoding(Some(charset));
        self.part
            .set_content_type(&content_type)
            .map_err(TextPartError::HeaderFormat)?;
        self.part.set_content(MimeContent::new(Cursor::new(bytes)));
        Ok(())
    }
}

impl<R: Read + Seek> TextPart<R> {
    /// Wraps an existing part after checking that its first Content-Type is text.
    ///
    /// Does not validate charset labels or body bytes, change headers, or perform
    /// I/O. Ownership follows the supplied part, including borrowed sources.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, malformed, or non-text Content-Type. On
    /// failure the supplied part is dropped; borrowed sources remain usable
    /// at their original positions.
    pub fn from_part(part: MimePart<R>) -> Result<Self, TextPartError> {
        let text = Self { part };
        text.checked_content_type()?;
        Ok(text)
    }

    /// Returns the underlying part, including raw metadata and stored content.
    pub fn part(&self) -> &MimePart<R> {
        &self.part
    }

    /// Returns mutable access to metadata, content, and serialization.
    ///
    /// Raw edits can remove or invalidate Content-Type. Subsequent text
    /// operations report that error rather than using stale metadata.
    pub fn part_mut(&mut self) -> &mut MimePart<R> {
        &mut self.part
    }

    /// Extracts the underlying part without copying or source I/O.
    pub fn into_part(self) -> MimePart<R> {
        self.part
    }

    /// Classifies the current Content-Type without reading or converting text.
    ///
    /// Recognizes flowed plain text, HTML, enriched text (including the older
    /// `text/richtext`), and RTF. Unknown text subtypes return Plain as in
    /// MimeKit, even though [`Self::is_plain`] is false for those subtypes.
    /// The format parameter is Unicode-trimmed before comparing with `flowed`.
    ///
    /// Uses the same case-insensitive matching as body selection and the
    /// predicates, including uppercase `PLAIN`, matching MimeKit.
    /// The fallback does not make unknown text types candidates for a plain
    /// message body. Charset labels and payload syntax are not examined.
    ///
    /// # Errors
    ///
    /// Missing, invalid, and non-text Content-Type return [`TextPartError`].
    /// In particular, this wrapper continues to reject `application/rtf`;
    /// ordinary MIME leaves and RichText body selection can still represent it.
    pub fn format(&self) -> Result<TextFormat, TextPartError> {
        let content_type = self.checked_content_type()?;
        // Flowed is both Plain and Flowed: classify the more specific format
        // first without narrowing plain-body selection to fixed-width text.
        Ok([
            TextFormat::Flowed,
            TextFormat::Html,
            TextFormat::Enriched,
            TextFormat::RichText,
        ]
        .into_iter()
        .find(|format| format.matches(&content_type))
        .unwrap_or(TextFormat::Plain))
    }

    /// Returns whether the current content type is `text/plain`, ignoring case.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, invalid, or non-text Content-Type.
    pub fn is_plain(&self) -> Result<bool, TextPartError> {
        Ok(TextFormat::Plain.matches(&self.checked_content_type()?))
    }

    /// Returns whether this is plain text with a trimmed `format=flowed` value.
    ///
    /// Matching is case-insensitive. Flowed text also satisfies [`Self::is_plain`].
    /// This does not validate flowed syntax or inspect the `delsp` parameter.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, invalid, or non-text Content-Type.
    pub fn is_flowed(&self) -> Result<bool, TextPartError> {
        Ok(TextFormat::Flowed.matches(&self.checked_content_type()?))
    }

    /// Returns whether the current content type is `text/html`, ignoring case.
    ///
    /// Does not inspect the body or interpret markup.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, invalid, or non-text Content-Type.
    pub fn is_html(&self) -> Result<bool, TextPartError> {
        Ok(TextFormat::Html.matches(&self.checked_content_type()?))
    }

    /// Returns whether this is `text/enriched` or its predecessor `text/richtext`.
    ///
    /// These are distinct from `text/rtf`; no content validation or conversion
    /// occurs. Matching is case-insensitive.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, invalid, or non-text Content-Type.
    pub fn is_enriched(&self) -> Result<bool, TextPartError> {
        Ok(TextFormat::Enriched.matches(&self.checked_content_type()?))
    }

    /// Returns whether the current type is `text/rtf`, ignoring case.
    ///
    /// Like other text-wrapper operations, rejects non-text media types,
    /// including `application/rtf`. It can instead remain an ordinary leaf
    /// selected with [`TextFormat::RichText`].
    ///
    /// # Errors
    ///
    /// Returns an error for missing, invalid, or non-text Content-Type.
    pub fn is_rich_text(&self) -> Result<bool, TextPartError> {
        Ok(TextFormat::RichText.matches(&self.checked_content_type()?))
    }

    /// Reads text using the declared charset, or UTF-8 when absent.
    ///
    /// Charset decoding follows transfer decoding using the stored content's
    /// encoding, not the requested Content-Transfer-Encoding header. There is
    /// no automatic recovery from unsupported labels or malformed bytes.
    /// Newlines and explicit BOMs are preserved.
    ///
    /// # Errors
    ///
    /// Metadata/charset-label errors precede I/O. Copying follows
    /// [`MimeContent::decode_to`]'s seek/error behavior. Successful copies rewind
    /// the source before charset decoding, even if decoding subsequently fails.
    pub fn text(&mut self) -> Result<String, TextPartError> {
        let content_type = self.checked_content_type()?;
        let charset = content_type
            .charset_encoding()
            .map_err(TextPartError::Charset)?
            .unwrap_or(Charset::UTF8);
        decode_text(&mut self.part, charset)
    }

    /// Reads text using an explicit charset instead of the Content-Type label.
    ///
    /// Still requires a valid text Content-Type, but an unknown or incorrect
    /// charset parameter can be overridden without editing raw headers.
    /// Strict decoding preserves newlines and explicit BOMs.
    ///
    /// # Errors
    ///
    /// Returns metadata, I/O, or charset errors as in [`Self::text`].
    pub fn text_with_charset(&mut self, charset: Charset) -> Result<String, TextPartError> {
        self.checked_content_type()?;
        decode_text(&mut self.part, charset)
    }

    fn checked_content_type(&self) -> Result<ContentType, TextPartError> {
        let content_type = self
            .part
            .content_type()
            .map_err(TextPartError::ContentType)?
            .ok_or(TextPartError::MissingContentType)?;
        if !content_type.is_mime_type("text", "*") {
            return Err(TextPartError::NotText(content_type.mime_type()));
        }
        Ok(content_type)
    }
}

pub(crate) fn decode_text<R: Read + Seek>(
    part: &mut MimePart<R>,
    charset: Charset,
) -> Result<String, TextPartError> {
    let mut bytes = Vec::new();
    part.content_mut()
        .decode_to(&mut bytes)
        .map_err(TextPartError::Io)?;
    charset
        .decode(&bytes)
        .map(|text| text.into_owned())
        .map_err(TextPartError::Charset)
}

/// A text part could not be constructed, read, or edited.
#[derive(Debug)]
pub enum TextPartError {
    /// No Content-Type header is present.
    MissingContentType,
    /// The first Content-Type has a non-text media type.
    NotText(String),
    /// A media subtype or the first raw Content-Type is invalid.
    ContentType(ContentHeaderParseError),
    /// Updated Content-Type metadata could not be formatted.
    HeaderFormat(HeaderFormatError),
    /// A charset label or character conversion failed.
    Charset(CharsetError),
    /// Reading or repositioning the stored body failed.
    Io(io::Error),
}

impl fmt::Display for TextPartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("text part has no Content-Type header"),
            Self::NotText(media_type) => write!(f, "expected a text media type, got {media_type}"),
            Self::ContentType(error) => error.fmt(f),
            Self::HeaderFormat(error) => error.fmt(f),
            Self::Charset(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl Error for TextPartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingContentType | Self::NotText(_) => None,
            Self::ContentType(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
            Self::Charset(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

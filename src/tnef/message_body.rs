// Adapted from MimeKit's TnefMessageBody.cs, TnefMessageBodyFormat.cs,
// and the body-format/encoding helpers in TnefMessageLoader.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    error::Error,
    fmt,
    io::{self, Cursor, Read},
};

use crate::{
    charsets::{self, Charset, CharsetError},
    io::FilteredReader,
};

use super::{RtfCompressedToRtf, TnefPropertyTag as Tag, TnefPropertyType as Type, TnefReadError};

/// The format of an owned TNEF message body.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TnefMessageBodyFormat {
    /// Plain text from a Body property or the legacy Body attribute.
    Text,
    /// HTML from a BodyHtml property.
    Html,
    /// Compressed RTF, as specified by [MS-OXRTFCP].
    CompressedRtf,
}

impl TnefMessageBodyFormat {
    /// Recognizes supported single-valued body tags, not arbitrary text properties.
    pub fn from_tag(tag: Tag) -> Option<Self> {
        if tag.is_multi_valued() {
            return None;
        }
        match (tag.id(), tag.value_type()) {
            (0x1000, Type::STRING8 | Type::UNICODE) => Some(Self::Text),
            (0x1013, Type::STRING8 | Type::UNICODE | Type::BINARY) => Some(Self::Html),
            (0x1009, Type::BINARY) => Some(Self::CompressedRtf),
            _ => None,
        }
    }
}

/// Failure to decompress or strictly decode the text of a TNEF body.
#[derive(Debug)]
pub enum TnefBodyError {
    /// A decoded reader failed. Compressed-RTF errors are available as the
    /// inner [`super::RtfDecompressionError`] of an `InvalidData` I/O error.
    Io(io::Error),
    /// The bytes are invalid in the body's selected or fallback charset.
    Charset(CharsetError),
}

impl fmt::Display for TnefBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Charset(error) => error.fmt(f),
        }
    }
}

impl Error for TnefBodyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Charset(error) => Some(error),
        }
    }
}

impl From<io::Error> for TnefBodyError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CharsetError> for TnefBodyError {
    fn from(error: CharsetError) -> Self {
        Self::Charset(error)
    }
}

/// An owned, repeatably readable plain-text, HTML, or compressed-RTF body.
///
/// String8 and aligned UTF-16 bodies have trailing NUL characters removed;
/// binary HTML and compressed RTF retain their exact bytes. Reader collection
/// obeys the parent's owned-byte limits before trimming. Direct construction
/// takes already-owned bytes and imposes no additional size limit.
///
/// This does not render RTF, extract encapsulated HTML, or select among duplicate
/// or legacy/MAPI bodies. Those are responsibilities of the message loader.
///
/// ```
/// use mimecraft::{charsets::Charset, tnef::{TnefMessageBody, TnefPropertyTag}};
///
/// let body = TnefMessageBody::new(
///     TnefPropertyTag::BODY_A, b"hello\0".to_vec(), Charset::UTF8,
/// )?;
/// assert_eq!(body.as_bytes(), b"hello");
/// assert_eq!(body.text()?, "hello");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug)]
pub struct TnefMessageBody {
    content: Vec<u8>,
    format: TnefMessageBodyFormat,
    tag: Tag,
    charset: Option<Charset>,
    fallback_charset: Charset,
}

impl TnefMessageBody {
    /// Takes raw body content and the current message charset without copying.
    /// Use `BODY_A` for a legacy Body attribute. Binary HTML initially has no
    /// known charset; call [`set_message_charsets`](Self::set_message_charsets)
    /// once the message's InternetCodepage is known.
    ///
    /// # Errors
    ///
    /// Rejects unsupported body tags and multivalued properties.
    pub fn new(
        tag: Tag,
        mut content: Vec<u8>,
        message_charset: Charset,
    ) -> Result<Self, TnefReadError> {
        let format =
            TnefMessageBodyFormat::from_tag(tag).ok_or(TnefReadError::InvalidPropertyType(tag))?;
        let unit = match tag.value_type() {
            Type::STRING8 => 1,
            Type::UNICODE => 2,
            _ => 0,
        };
        // Remove trailing null characters from text bodies. An incomplete
        // UTF-16 code unit must not cause us to trim individual zero bytes.
        if unit != 0 && content.len().is_multiple_of(unit) {
            while content.len() >= unit
                && content[content.len() - unit..]
                    .iter()
                    .all(|&byte| byte == 0)
            {
                content.truncate(content.len() - unit);
            }
        }
        let mut body = Self {
            content,
            format,
            tag,
            charset: None,
            fallback_charset: message_charset,
        };
        body.set_message_charsets(message_charset, None);
        Ok(body)
    }

    /// The format of this body.
    pub fn format(&self) -> TnefMessageBodyFormat {
        self.format
    }

    /// The original body property tag (`BODY_A` for a legacy Body attribute).
    pub fn tag(&self) -> Tag {
        self.tag
    }

    /// The charset of the raw content, if known. Unicode is always UTF-16LE;
    /// compressed RTF has no declared charset.
    pub fn charset(&self) -> Option<Charset> {
        self.charset
    }

    /// Applies the message's final charset and optional InternetCodepage charset.
    ///
    /// String8 uses the message charset, Unicode remains UTF-16LE, and Binary
    /// HTML uses only the InternetCodepage charset. Compressed RTF remains
    /// unspecified. Text access uses the message charset when unspecified.
    /// No BOM or HTML meta-charset detection is performed.
    pub fn set_message_charsets(
        &mut self,
        message_charset: Charset,
        internet_charset: Option<Charset>,
    ) {
        // The codepage may have changed after the body was read if there was
        // no attOemCodepage and PidTagInternetCodepage followed the body.
        // See [MS-OXTNEF] section 2.3.3.2.
        self.fallback_charset = message_charset;
        self.charset = match self.tag.value_type() {
            Type::UNICODE => Some(charsets::from_codepage(1200).expect("UTF-16LE is supported")),
            Type::STRING8 => Some(message_charset),
            _ if self.format == TnefMessageBodyFormat::Html => internet_charset,
            _ => None,
        };
    }

    /// The raw content after text terminators have been removed.
    pub fn as_bytes(&self) -> &[u8] {
        &self.content
    }

    /// Raw byte length, after text terminators have been removed.
    pub fn len(&self) -> usize {
        self.content.len()
    }

    /// Whether the raw content is empty.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Opens an independent, seekable cursor over the raw content.
    pub fn open_read(&self) -> Cursor<&[u8]> {
        Cursor::new(self.as_bytes())
    }

    /// Opens an independent decoded reader. Only compressed RTF is transformed.
    ///
    /// Unlike MimeKit's tolerant filtered stream, this reader validates RTF
    /// framing/compression/CRC at EOF and reports failures as `InvalidData`.
    /// Previously returned bytes remain provisional until a nonempty read
    /// returns zero successfully. Dropping early performs no validation.
    pub fn open_decoded_read(&self) -> TnefBodyReader<'_> {
        let inner = if self.format == TnefMessageBodyFormat::CompressedRtf {
            BodyReader::Rtf(Box::new(FilteredReader::new(
                self.as_bytes(),
                RtfCompressedToRtf::new(),
            )))
        } else {
            BodyReader::Raw(self.open_read())
        };
        TnefBodyReader { inner }
    }

    fn decoded_bytes(&self) -> io::Result<Cow<'_, [u8]>> {
        if self.format != TnefMessageBodyFormat::CompressedRtf {
            return Ok(Cow::Borrowed(self.as_bytes()));
        }
        // Grow with actual decoded output, not the untrusted RAWSIZE field.
        let mut bytes = Vec::new();
        self.open_decoded_read().read_to_end(&mut bytes)?;
        Ok(Cow::Owned(bytes))
    }

    /// Decodes text strictly, without BOM detection or RTF rendering.
    /// Like [`Charset::decode`], preserves a leading BOM as a character.
    ///
    /// # Errors
    ///
    /// Returns decompression/framing/CRC errors or invalid charset bytes.
    pub fn text(&self) -> Result<Cow<'_, str>, TnefBodyError> {
        let charset = self.charset.unwrap_or(self.fallback_charset);
        Ok(match self.decoded_bytes()? {
            Cow::Borrowed(bytes) => charset.decode(bytes)?,
            Cow::Owned(bytes) => Cow::Owned(charset.decode(&bytes)?.into_owned()),
        })
    }

    /// Decodes text with explicit U+FFFD recovery reporting, without BOM
    /// detection or RTF rendering. Preserves a leading BOM as a character.
    /// The boolean indicates replaced input.
    ///
    /// # Errors
    ///
    /// Compressed RTF must still pass framing/compression/CRC validation.
    pub fn text_lossy(&self) -> io::Result<(Cow<'_, str>, bool)> {
        let charset = self.charset.unwrap_or(self.fallback_charset);
        Ok(match self.decoded_bytes()? {
            Cow::Borrowed(bytes) => charset.decode_lossy(bytes),
            Cow::Owned(bytes) => {
                let (text, replaced) = charset.decode_lossy(&bytes);
                (Cow::Owned(text.into_owned()), replaced)
            }
        })
    }
}

enum BodyReader<'a> {
    Raw(Cursor<&'a [u8]>),
    Rtf(Box<FilteredReader<&'a [u8], RtfCompressedToRtf>>),
}

/// A repeatable body reader with automatic compressed-RTF validation at EOF.
///
/// Output is provisional until EOF succeeds. Empty reads and dropping do not
/// validate. RTF validation errors repeat on subsequent nonempty reads.
pub struct TnefBodyReader<'a> {
    inner: BodyReader<'a>,
}

impl Read for TnefBodyReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        match &mut self.inner {
            BodyReader::Raw(reader) => reader.read(buffer),
            BodyReader::Rtf(reader) => {
                let count = reader.read(buffer)?;
                if count == 0 {
                    reader
                        .filter()
                        .validate()
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                }
                Ok(count)
            }
        }
    }
}

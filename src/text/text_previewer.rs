// Adapted from MimeKit's Text/TextPreviewer.cs and Utils/CharsetUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek};

use crate::{
    TextPart, TextPartError,
    charsets::{Charset, CharsetError, bom_charset},
};

use super::{
    HtmlTextPreviewer, InvalidPreviewLength, PlainTextPreviewer,
    plain_text_previewer::validate_preview_length,
};

/// Produces best-effort plain-text previews of MIME text parts.
///
/// Reads at most the first 16 KiB of transfer-decoded content, then dispatches
/// HTML to [`HtmlTextPreviewer`] and every other text format to
/// [`PlainTextPreviewer`]. HTML requires an explicit `body` element. Flowed,
/// enriched, and RTF text is not converted between formats.
///
/// A supported declared charset wins and uses replacement decoding.
/// Otherwise UTF-16LE/BE or UTF-8 BOMs select the encoding, defaulting to
/// strict UTF-8. Invalid UTF-8 encountered by the previewer retries the
/// original prefix as Latin-1. A matching initial BOM is removed, but a
/// conflicting BOM never overrides a supported declaration. No HTML charset
/// sniffing or automatic message-body selection is performed.
///
/// Unlike .NET's charset-dependent replacements, malformed declared input
/// uniformly uses U+FFFD. Undeclared UTF-8 validation follows the existing
/// preview readers, not .NET's character-buffer read-ahead: unneeded suffix
/// bytes may remain unvalidated. A cutoff splitting a character uses the
/// same replacement/fallback rules as malformed input.
///
/// The default output limit is 230; valid limits are 1 through 1024 Unicode
/// scalars. Output is plain text, not safe HTML; escape it before embedding.
/// Unlike [`TextPart::text`], previewing does not sniff HTML meta declarations.
///
/// ```
/// use mimecraft::{TextPart, text::TextPreviewer};
///
/// let mut body = TextPart::new("html", "<body><p>Hello</p><p>world</p></body>")?;
/// assert_eq!(TextPreviewer::new().preview(&mut body)?, "Hello world");
/// # Ok::<(), mimecraft::TextPartError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextPreviewer {
    maximum_preview_length: usize,
}

impl Default for TextPreviewer {
    fn default() -> Self {
        Self {
            maximum_preview_length: 230,
        }
    }
}

impl TextPreviewer {
    /// Creates a previewer with a maximum length of 230 Unicode scalar values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a previewer with an explicit maximum output length.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPreviewLength`] if the limit is outside 1..=1024.
    pub fn with_maximum_preview_length(
        maximum_preview_length: usize,
    ) -> Result<Self, InvalidPreviewLength> {
        validate_preview_length(maximum_preview_length)?;
        Ok(Self {
            maximum_preview_length,
        })
    }

    /// Returns the maximum preview length in Unicode scalar values.
    pub fn maximum_preview_length(&self) -> usize {
        self.maximum_preview_length
    }

    /// Generates a preview using the part's stored transfer encoding.
    ///
    /// Preserves metadata and stored bytes. Rewinds before reading but not
    /// afterward; source read-ahead can exceed the decoded byte limit.
    /// The entire prefix is read before previewing, so early preview completion
    /// does not hide source errors while collecting it. The source is not
    /// closed, and subsequent calls reopen it from the start.
    ///
    /// # Errors
    ///
    /// Returns metadata, seek, or read errors without a partial preview.
    /// Unsupported charset labels intentionally use the recovery path
    /// documented on [`TextPreviewer`].
    pub fn preview<R: Read + Seek>(&self, body: &mut TextPart<R>) -> Result<String, TextPartError> {
        let content_type = body.checked_content_type()?;
        let charset = match content_type.charset_encoding() {
            Ok(charset) => charset,
            Err(CharsetError::UnsupportedLabel(_)) => None,
            Err(error) => return Err(TextPartError::Charset(error)),
        };
        let mut bytes = Vec::with_capacity(16 * 1024);
        // Bound transfer-decoded bytes, not encoded source bytes. Opening also
        // restarts codec state when an earlier preview stopped mid-stream.
        // Avoid content_mut() so parsed MIME framing remains unchanged.
        body.part_mut()
            .content
            .open()
            .map_err(TextPartError::Io)?
            .take(16 * 1024)
            .read_to_end(&mut bytes)
            .map_err(TextPartError::Io)?;
        self.preview_bytes(&bytes, charset, content_type.is_mime_type("text", "html"))
            .map_err(TextPartError::Io)
    }

    fn preview_bytes(
        &self,
        bytes: &[u8],
        declared: Option<Charset>,
        html: bool,
    ) -> io::Result<String> {
        let preview = |mut input: &[u8]| {
            if html {
                HtmlTextPreviewer::with_maximum_preview_length(self.maximum_preview_length)
                    .expect("validated preview length")
                    .preview_reader(&mut input)
            } else {
                PlainTextPreviewer::with_maximum_preview_length(self.maximum_preview_length)
                    .expect("validated preview length")
                    .preview_reader(&mut input)
            }
        };
        // Only missing/unsupported declarations permit BOM-based selection.
        let charset = declared
            .or_else(|| bom_charset(bytes))
            .unwrap_or(Charset::UTF8);
        let preamble: &[u8] = match charset.name() {
            "utf-8" => b"\xef\xbb\xbf",
            "utf-16" => b"\xff\xfe",
            "utf-16be" => b"\xfe\xff",
            _ => b"",
        };
        let input = bytes.strip_prefix(preamble).unwrap_or(bytes);
        if declared.is_some() || charset != Charset::UTF8 {
            return preview(charset.decode_lossy(input).0.as_bytes());
        }
        match preview(input) {
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                // Retry the original bytes, including any BOM, just as resetting
                // the source stream and reopening it as Latin-1 does upstream.
                preview(Charset::LATIN1.decode_lossy(bytes).0.as_bytes())
            }
            result => result,
        }
    }
}

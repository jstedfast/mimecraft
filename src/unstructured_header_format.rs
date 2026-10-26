// Adapted from MimeKit's Header.cs and Utils/Rfc2047.cs (TokenFolder).
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, error::Error, fmt};

use crate::{
    encodings::{InvalidLineLength, NewLineFormat},
    rfc2047::{self, EncodeError, EncodeOptions},
    utils::unfold,
};

/// Options for complete unstructured headers, such as Subject.
///
/// Defaults to 78-byte physical lines, CRLF, and UTF-8 encoded words.
/// Unlike MimeKit's soft folding target, the physical-line limit is enforced.
/// Output remains ASCII unless [`Self::international`] is enabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnstructuredHeaderFormatOptions {
    /// Maximum bytes per physical line, excluding its newline, in `60..=998`.
    ///
    /// The field name and colon must fit. Continuation spaces count as one byte.
    pub max_line_length: usize,
    /// DOS (CRLF) or Unix (LF) endings. `Mixed` is rejected.
    pub new_line_format: NewLineFormat,
    /// Charset and encoded-word limit for the text.
    ///
    /// Validated before capping the word limit at `max_line_length - 1`, so each
    /// word fits with a continuation space. Encoding errors refer to the
    /// normalized (trimmed and unfolded) input, not the original string.
    /// In international mode, the charset applies only when encoded words are
    /// needed; raw text is always UTF-8. The word limit is always validated.
    pub encoding: EncodeOptions,
    /// Allow raw UTF-8 text, defaulting to false.
    ///
    /// Folds at existing spaces, measuring UTF-8 bytes rather than characters.
    /// Controls, literal encoded-word lookalikes, or a token too long for a
    /// continuation line retain whole-value RFC 2047 encoding with [`Self::encoding`].
    /// Like MimeKit, this avoids splitting a token and inserting whitespace
    /// into its decoded value.
    ///
    /// For SMTP delivery, generally set this to match whether the server
    /// advertises `SMTPUTF8` in its `EHLO` response. Leave it false if that
    /// extension is unavailable; enabling it is optional when supported.
    ///
    /// This does not enable international address formatting or negotiate
    /// transport capabilities. Callers must ensure the destination accepts
    /// UTF-8 headers.
    ///
    /// ```
    /// use mimecraft::{Header, UnstructuredHeaderFormatOptions};
    ///
    /// let options = UnstructuredHeaderFormatOptions {
    ///     international: true,
    ///     ..Default::default()
    /// };
    /// let header = Header::from_unstructured_with_options("Subject", "caf\u{e9}", &options)?;
    /// assert_eq!(header.raw(), "Subject: caf\u{e9}\r\n".as_bytes());
    /// # Ok::<(), mimecraft::UnstructuredHeaderFormatError>(())
    /// ```
    pub international: bool,
}

impl Default for UnstructuredHeaderFormatOptions {
    fn default() -> Self {
        Self {
            max_line_length: 78,
            new_line_format: NewLineFormat::Dos,
            encoding: EncodeOptions::default(),
            international: false,
        }
    }
}

/// A complete unstructured header could not be formatted.
///
/// No partial header is returned and no source value is modified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnstructuredHeaderFormatError {
    /// The field is empty or contains whitespace, controls, a colon, or non-ASCII.
    InvalidFieldName {
        /// Invalid UTF-8 byte offset, or zero for an empty name.
        offset: usize,
    },
    /// The physical-line limit is outside `60..=998`.
    InvalidLineLength(InvalidLineLength),
    /// Mixed line endings cannot be emitted.
    InvalidNewLineFormat(NewLineFormat),
    /// The text cannot be encoded with the selected charset or word limit.
    Encoding(EncodeError),
    /// The field name and colon cannot fit on one line.
    LineTooLong {
        /// Bytes required for the field name and colon.
        required: usize,
        /// The configured physical-line limit.
        limit: usize,
    },
}

impl fmt::Display for UnstructuredHeaderFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFieldName { offset } => write!(f, "invalid field name at byte {offset}"),
            Self::InvalidLineLength(error) => error.fmt(f),
            Self::InvalidNewLineFormat(format) => {
                write!(
                    f,
                    "header new-line format must be Dos or Unix, got {format:?}"
                )
            }
            Self::Encoding(error) => error.fmt(f),
            Self::LineTooLong { required, limit } => {
                write!(
                    f,
                    "header field needs {required} bytes but the line limit is {limit}"
                )
            }
        }
    }
}

impl Error for UnstructuredHeaderFormatError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidLineLength(error) => Some(error),
            Self::Encoding(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) fn format_header(
    field: &str,
    value: &str,
    options: &UnstructuredHeaderFormatOptions,
) -> Result<String, UnstructuredHeaderFormatError> {
    if field.is_empty() {
        return Err(UnstructuredHeaderFormatError::InvalidFieldName { offset: 0 });
    }
    if let Some(offset) = field
        .bytes()
        .position(|byte| !(33..=126).contains(&byte) || byte == b':')
    {
        return Err(UnstructuredHeaderFormatError::InvalidFieldName { offset });
    }
    if !(60..=998).contains(&options.max_line_length) {
        return Err(UnstructuredHeaderFormatError::InvalidLineLength(
            InvalidLineLength::new(options.max_line_length),
        ));
    }
    let newline = match options.new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => {
            return Err(UnstructuredHeaderFormatError::InvalidNewLineFormat(
                NewLineFormat::Mixed,
            ));
        }
    };
    if !(1..=75).contains(&options.encoding.max_word_length) {
        return Err(UnstructuredHeaderFormatError::Encoding(
            EncodeError::InvalidWordLength {
                requested: options.encoding.max_word_length,
            },
        ));
    }
    let mut column = field.len() + 1;
    if column > options.max_line_length {
        return Err(UnstructuredHeaderFormatError::LineTooLong {
            required: column,
            limit: options.max_line_length,
        });
    }

    // Header.SetValue trims and unfolds before encoding, without interpreting
    // the assigned text as pre-encoded words.
    let normalized = unfold(value);
    let encoding = EncodeOptions {
        max_word_length: options
            .encoding
            .max_word_length
            .min(options.max_line_length - 1),
        ..options.encoding
    };
    let encoded = if options.international
        && !normalized.bytes().any(|byte| byte.is_ascii_control())
        && !normalized.contains("=?")
        && normalized
            .split(' ')
            .all(|word| word.len() < options.max_line_length)
    {
        normalized
    } else {
        // Folding inside a word would insert whitespace into its decoded value.
        // Encode the whole value so original whitespace stays in the payloads,
        // not in the ignored separators between adjacent encoded words.
        // Each encoded word must fit with its indentation.
        // Controls and literal encoded-word text keep this whole-value policy
        // even in international mode, instead of changing the displayed value.
        Cow::Owned(
            rfc2047::encode_text_with_options(&normalized, &encoding)
                .map_err(UnstructuredHeaderFormatError::Encoding)?,
        )
    };
    let mut output = format!("{field}:");
    if !encoded.is_empty() {
        for word in encoded.split(' ') {
            if column + 1 + word.len() > options.max_line_length {
                // Break just before the last whitespace character, as in
                // TokenFolder. Keep spaces rather than replacing them with tabs:
                // unfolding removes the newline, not continuation whitespace.
                output.push_str(newline);
                column = 0;
            }
            output.push(' ');
            output.push_str(word);
            column += 1 + word.len();
        }
    }
    output.push_str(newline);
    Ok(output)
}

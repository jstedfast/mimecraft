// Adapted from MimeKit's InternetAddressList.cs, MailboxAddress.cs,
// GroupAddress.cs and FormatOptions.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::{
    AddressEncodeError, AddressList,
    encodings::{InvalidLineLength, NewLineFormat},
    rfc2047::EncodeOptions,
};

/// Options for folded address headers.
///
/// Defaults to 78-byte physical lines, CRLF endings, and UTF-8 encoded names.
/// Unlike MimeKit's soft wrapping target, the line limit is enforced.
/// Output is ASCII unless [`Self::international`] is enabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressHeaderFormatOptions {
    /// Maximum bytes per physical line, excluding its newline, in `60..=998`.
    ///
    /// A continuation tab counts as one byte. Addr-specs, source routes, quoted
    /// strings, and attached punctuation are not split to fit smaller limits.
    pub max_line_length: usize,
    /// DOS (CRLF) or Unix (LF) endings. `Mixed` is rejected.
    pub new_line_format: NewLineFormat,
    /// Charset and encoded-word limit for all display names.
    ///
    /// The word limit is additionally capped at `max_line_length - 2`, leaving
    /// room for a continuation tab and a group's colon. Invalid options are
    /// rejected before applying this cap.
    /// In international mode, the charset is used only for names requiring
    /// encoding; raw names are UTF-8. The word limit is always validated.
    pub name_encoding: EncodeOptions,
    /// Allow UTF-8 names and local parts, defaulting to false.
    ///
    /// Retains stored domain and route spelling, including ACE labels, after
    /// the same domain validation used for ASCII output. Domain literals retain
    /// the existing whitespace normalization; no DNS or delivery checks occur.
    /// Raw name pieces must fit with a continuation tab and a group's colon.
    /// Oversized pieces, controls, or literal encoded-word lookalikes cause the
    /// whole name to use the selected encoding, preserving its decoded text.
    ///
    /// Addr-specs and source routes remain indivisible; oversized pieces fail.
    /// This does not negotiate SMTPUTF8 or change the ASCII-only value APIs.
    /// Callers must ensure the destination accepts internationalized headers
    /// and addresses.
    ///
    /// ```
    /// use mimecraft::{AddressList, AddressHeaderFormatOptions};
    ///
    /// let list: AddressList = "Ren\u{e9}e <caf\u{e9}@example.com>".parse()?;
    /// let options = AddressHeaderFormatOptions {
    ///     international: true,
    ///     ..Default::default()
    /// };
    /// assert_eq!(list.to_header_string_with_options("To", &options)?,
    ///     "To: Ren\u{e9}e <caf\u{e9}@example.com>\r\n");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub international: bool,
}

impl Default for AddressHeaderFormatOptions {
    fn default() -> Self {
        Self {
            max_line_length: 78,
            new_line_format: NewLineFormat::Dos,
            name_encoding: EncodeOptions::default(),
            international: false,
        }
    }
}

/// A complete address header could not be serialized.
///
/// No partial header is returned, and the source addresses are unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AddressHeaderFormatError {
    /// A field name was empty or contained whitespace, controls, a colon, or non-ASCII text.
    InvalidFieldName {
        /// Original UTF-8 byte offset of the invalid byte, or zero for an empty name.
        offset: usize,
    },
    /// The requested line limit is outside `60..=998`.
    InvalidLineLength(InvalidLineLength),
    /// Mixed line endings cannot be emitted.
    InvalidNewLineFormat(NewLineFormat),
    /// An address or name could not be encoded; includes nested member indexes.
    Address(AddressEncodeError),
    /// A field prefix or indivisible value piece exceeds the physical-line limit.
    LineTooLong {
        /// Bytes needed, including the colon or continuation indentation.
        required: usize,
        /// The requested physical-line limit.
        limit: usize,
    },
}

impl fmt::Display for AddressHeaderFormatError {
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
            Self::Address(error) => error.fmt(f),
            Self::LineTooLong { required, limit } => {
                write!(
                    f,
                    "header piece needs {required} bytes but the line limit is {limit}"
                )
            }
        }
    }
}

impl Error for AddressHeaderFormatError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidLineLength(error) => Some(error),
            Self::Address(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) fn format_header(
    list: &AddressList,
    field: &str,
    options: &AddressHeaderFormatOptions,
) -> Result<String, AddressHeaderFormatError> {
    if field.is_empty() {
        return Err(AddressHeaderFormatError::InvalidFieldName { offset: 0 });
    }
    if let Some(offset) = field
        .bytes()
        .position(|byte| !(33..=126).contains(&byte) || byte == b':')
    {
        return Err(AddressHeaderFormatError::InvalidFieldName { offset });
    }
    if !(60..=998).contains(&options.max_line_length) {
        return Err(AddressHeaderFormatError::InvalidLineLength(
            InvalidLineLength::new(options.max_line_length),
        ));
    }
    let newline = match options.new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => {
            return Err(AddressHeaderFormatError::InvalidNewLineFormat(
                NewLineFormat::Mixed,
            ));
        }
    };
    let mut writer = HeaderWriter {
        output: format!("{field}:"),
        column: field.len() + 1,
        limit: options.max_line_length,
        newline,
    };
    writer.check_length(writer.column)?;
    crate::address_encode::validate_options(&options.name_encoding)
        .map_err(AddressHeaderFormatError::Address)?;
    let encoding = EncodeOptions {
        max_word_length: options
            .name_encoding
            .max_word_length
            .min(options.max_line_length - 2),
        ..options.name_encoding
    };
    let value = crate::address_encode::encode_list(
        list,
        &encoding,
        options.international,
        options.max_line_length - 2,
    )
    .map_err(AddressHeaderFormatError::Address)?;
    if !value.is_empty() {
        append_value(&value, &mut writer)?;
    }
    writer.output.push_str(newline);
    Ok(writer.output)
}

struct HeaderWriter {
    output: String,
    column: usize,
    limit: usize,
    newline: &'static str,
}

impl HeaderWriter {
    fn check_length(&self, required: usize) -> Result<(), AddressHeaderFormatError> {
        if required > self.limit {
            Err(AddressHeaderFormatError::LineTooLong {
                required,
                limit: self.limit,
            })
        } else {
            Ok(())
        }
    }

    fn piece(&mut self, piece: &str) -> Result<(), AddressHeaderFormatError> {
        let required = 1 + piece.len();
        self.check_length(required)?;
        if self.column + required > self.limit {
            // The piece fits on a line of its own, but not on the current line.
            self.output.push_str(self.newline);
            self.output.push('\t');
            self.column = required;
        } else {
            self.output.push(' ');
            self.column += required;
        }
        self.output.push_str(piece);
        Ok(())
    }
}

fn append_value(value: &str, writer: &mut HeaderWriter) -> Result<(), AddressHeaderFormatError> {
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut literal = false;
    let mut angled = false;
    // Only inspect syntax produced by our encoder. Its single spaces outside
    // quotes/angles/literals are safe fold points; whitespace inside a quoted
    // local part or name is data and must not be replaced by a continuation tab.
    for (index, byte) in value.bytes().enumerate() {
        if literal {
            if byte == b']' {
                literal = false;
            }
        } else if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' => literal = true,
                b'<' => angled = true,
                b'>' => angled = false,
                b' ' if !angled => {
                    writer.piece(&value[start..index])?;
                    start = index + 1;
                }
                _ => {}
            }
        }
    }
    writer.piece(&value[start..])
}

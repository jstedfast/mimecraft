// Adapted from MimeKit's Received.cs and FormatOptions.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, error::Error, fmt};

use crate::{
    Received, ReceivedClauseId,
    encodings::{InvalidLineLength, NewLineFormat},
    received::unfold,
};

/// Options for complete Received headers.
///
/// Defaults to 78-byte physical lines, CRLF, and ASCII output. Unlike MimeKit's
/// soft folding target, the byte limit is enforced. Keywords, complete values,
/// comments, and the date are indivisible; formatting never splits their text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceivedFormatOptions {
    /// Maximum bytes per physical line, excluding its newline, in `60..=998`.
    ///
    /// The field name, colon, continuation tabs, and comment escapes count.
    pub max_line_length: usize,
    /// DOS (CRLF) or Unix (LF) endings. `Mixed` is rejected.
    pub new_line_format: NewLineFormat,
    /// Allow raw UTF-8, defaulting to false.
    ///
    /// For SMTP delivery, generally set this to match whether the server
    /// advertises `SMTPUTF8` in its `EHLO` response. Leave it false if that
    /// extension is unavailable; enabling it is optional when supported.
    ///
    /// No IDNA conversion or encoded-word encoding is performed. Non-ASCII
    /// text is an error unless enabled. This does not negotiate SMTP extensions
    /// or validate trace data; callers must ensure the destination accepts UTF-8.
    pub international: bool,
}

impl Default for ReceivedFormatOptions {
    fn default() -> Self {
        Self {
            max_line_length: 78,
            new_line_format: NewLineFormat::Dos,
            international: false,
        }
    }
}

/// The stored component containing text that could not be formatted.
///
/// Clause and comment indexes are zero-based in their current collections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceivedFormatPart {
    /// A clause's keyword.
    Keyword(usize),
    /// A clause's value.
    Value(usize),
    /// One comment on a clause, including leading comments.
    Comment {
        /// Clause index.
        clause: usize,
        /// Comment index within the clause.
        comment: usize,
    },
    /// The retained or newly formatted date suffix.
    DateTime,
}

/// A complete Received header could not be formatted.
///
/// No partial output is returned and the source object is not modified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReceivedFormatError {
    /// The requested line limit is outside `60..=998`.
    InvalidLineLength(InvalidLineLength),
    /// Mixed line endings cannot be emitted.
    InvalidNewLineFormat(NewLineFormat),
    /// An indivisible piece cannot fit on a continuation line.
    LineTooLong {
        /// Bytes needed, including continuation indentation.
        required: usize,
        /// Requested physical-line limit.
        limit: usize,
    },
    /// An ASCII control cannot be emitted.
    ///
    /// Comment CR/LF/TAB are normalized instead; other controls are rejected.
    InvalidControl {
        /// Stored component containing the control.
        part: ReceivedFormatPart,
        /// UTF-8 byte offset in that component, before comment normalization.
        offset: usize,
    },
    /// Non-ASCII text requires explicit international formatting.
    NonAscii {
        /// Stored component containing non-ASCII text.
        part: ReceivedFormatPart,
        /// UTF-8 byte offset of the first non-ASCII character.
        offset: usize,
    },
}

impl fmt::Display for ReceivedFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineLength(error) => error.fmt(f),
            Self::InvalidNewLineFormat(format) => {
                write!(
                    f,
                    "Received new-line format must be Dos or Unix, got {format:?}"
                )
            }
            Self::LineTooLong { required, limit } => {
                write!(
                    f,
                    "Received piece needs {required} bytes but the line limit is {limit}"
                )
            }
            Self::InvalidControl { part, offset } => {
                write!(f, "control in Received {part:?} at byte {offset}")
            }
            Self::NonAscii { part, offset } => {
                write!(
                    f,
                    "non-ASCII Received {part:?} at byte {offset} requires international formatting"
                )
            }
        }
    }
}

impl Error for ReceivedFormatError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidLineLength(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) fn format_header(
    received: &Received,
    options: &ReceivedFormatOptions,
) -> Result<String, ReceivedFormatError> {
    if !(60..=998).contains(&options.max_line_length) {
        return Err(ReceivedFormatError::InvalidLineLength(
            InvalidLineLength::new(options.max_line_length),
        ));
    }
    let newline = match options.new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => {
            return Err(ReceivedFormatError::InvalidNewLineFormat(
                NewLineFormat::Mixed,
            ));
        }
    };
    let mut writer = HeaderWriter {
        output: "Received:".to_owned(),
        column: "Received:".len(),
        limit: options.max_line_length,
        newline,
    };
    for (index, clause) in received.clauses().iter().enumerate() {
        let comments = clause
            .comments()
            .iter()
            .enumerate()
            .map(|(comment, text)| {
                format_comment(
                    text,
                    ReceivedFormatPart::Comment {
                        clause: index,
                        comment,
                    },
                    options,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if clause.id() != ReceivedClauseId::None {
            let keyword = clause.keyword();
            let value = clause.value();
            validate_text(keyword, ReceivedFormatPart::Keyword(index), options)?;
            validate_text(value, ReceivedFormatPart::Value(index), options)?;
            let comment_length = comments.first().map_or(0, String::len);
            let following = if comment_length > 0
                && keyword.len() + 1 + value.len() + 1 + comment_length < writer.limit
            {
                // Keyword, value and the first comment can all fit on one line.
                // Fold before the keyword if necessary to keep them together.
                1 + value.len() + 1 + comment_length
            } else if keyword.len() + 1 + value.len() < writer.limit {
                // Keyword and value can both fit on one line; wrap the pair if
                // they are too long to fit on the current line.
                1 + value.len()
            } else {
                // We need to separate the keyword and value. Fold before the
                // keyword only if it cannot fit on the current line.
                0
            };
            // Append the keyword, then the value, folding between them if needed.
            writer.piece(keyword, following)?;
            writer.piece(value, 0)?;
        }
        for comment in comments {
            writer.piece(&comment, 0)?;
        }
    }
    if let Some(date) = received.date_time() {
        if writer.column + 1 > writer.limit {
            writer.fold();
        }
        writer.output.push(';');
        writer.column += 1;
        // Use the cached value if available so reformatting a parsed header
        // keeps its original date spelling intact.
        let stamp = received
            .date_time_text()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(date.to_string()));
        validate_text(&stamp, ReceivedFormatPart::DateTime, options)?;
        writer.piece(&stamp, 0)?;
    }
    writer.output.push_str(newline);
    Ok(writer.output)
}

fn validate_text(
    text: &str,
    part: ReceivedFormatPart,
    options: &ReceivedFormatOptions,
) -> Result<(), ReceivedFormatError> {
    for (offset, ch) in text.char_indices() {
        if ch.is_ascii_control()
            && !(matches!(part, ReceivedFormatPart::Comment { .. })
                && matches!(ch, '\r' | '\n' | '\t'))
        {
            return Err(ReceivedFormatError::InvalidControl { part, offset });
        }
        if !options.international && !ch.is_ascii() {
            return Err(ReceivedFormatError::NonAscii { part, offset });
        }
    }
    Ok(())
}

fn format_comment(
    text: &str,
    part: ReceivedFormatPart,
    options: &ReceivedFormatOptions,
) -> Result<String, ReceivedFormatError> {
    validate_text(text, part, options)?;
    // Comments are publicly editable, so ensure they cannot inject line breaks.
    let text = unfold(text);
    let mut depth = 0usize;
    let balanced = text.bytes().all(|byte| match byte {
        b'(' => {
            depth += 1;
            true
        }
        b')' if depth == 0 => false,
        b')' => {
            depth -= 1;
            true
        }
        _ => true,
    }) && depth == 0;
    let mut output = String::from("(");
    for ch in text.chars() {
        // Preserve balanced nested comments like MimeKit. Escape all parentheses
        // if they are unbalanced so edited text cannot close the outer comment.
        if ch == '\\' || (!balanced && matches!(ch, '(' | ')')) {
            output.push('\\');
        }
        output.push(ch);
    }
    output.push(')');
    Ok(output)
}

struct HeaderWriter {
    output: String,
    column: usize,
    limit: usize,
    newline: &'static str,
}

impl HeaderWriter {
    fn fold(&mut self) {
        self.output.push_str(self.newline);
        self.output.push('\t');
        self.column = 1;
    }

    fn piece(&mut self, text: &str, following: usize) -> Result<(), ReceivedFormatError> {
        let required = 1 + text.len();
        if required > self.limit {
            return Err(ReceivedFormatError::LineTooLong {
                required,
                limit: self.limit,
            });
        }
        if self.column + required + following > self.limit {
            self.fold();
        } else {
            // The piece (and any following group) fits on the current line.
            self.output.push(' ');
            self.column += 1;
        }
        self.output.push_str(text);
        self.column += text.len();
        Ok(())
    }
}

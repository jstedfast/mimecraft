// Adapted from MimeKit's Parameter.cs, FormatOptions.cs, ContentType.cs and
// ContentDisposition.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, iter::Peekable, str::Chars};

use crate::{
    Parameter, ParameterEncodingMethod, ParameterList,
    charsets::{Charset, CharsetError},
    encodings::{InvalidLineLength, NewLineFormat, Rfc2231Encoder},
    parse_utils::is_attribute,
    rfc2047,
    utils::quote,
};

/// Options for Content-Type and Content-Disposition serialization.
///
/// Defaults to 78-byte lines, CRLF endings, and quoting only when necessary.
/// RFC 2231 is the default for non-ASCII text, controls, encoded-word lookalikes,
/// and long values. RFC 2047 compatibility output can be explicitly selected or
/// recovered from parsed parameters. Output charset selection prefers ASCII
/// and Latin-1, then the configured charset (UTF-8 by default).
/// Output is ASCII unless [`Self::international`] enables UTF-8 parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderFormatOptions {
    /// Maximum bytes per physical line, excluding the newline, in `60..=998`.
    ///
    /// Unlike MimeKit's soft target, this is enforced. An unbreakable header
    /// value, parameter name, or encoded character that cannot fit is an error.
    pub max_line_length: usize,
    /// DOS (CRLF) or Unix (LF) endings. `Mixed` is rejected.
    pub new_line_format: NewLineFormat,
    /// Quote ordinary values and unencoded continuation sections even if they
    /// contain only attribute characters. Extended values remain percent encoded.
    pub always_quote_parameters: bool,
    /// Encoding for parameters that cannot use ordinary tokens/quoted strings.
    ///
    /// Defaults to RFC 2231. A parameter's explicit override takes precedence.
    /// RFC 2047 is a legacy compatibility convention, not standard MIME parameter
    /// syntax. Both methods enforce the physical-line limit and preserve literal
    /// values rather than interpreting existing encoded words.
    pub parameter_encoding_method: ParameterEncodingMethod,
    /// Preferred charset for parameter text containing characters above U+00FF.
    ///
    /// Defaults to UTF-8. A parameter's charset preference takes precedence.
    /// Printable ASCII still selects ASCII; other text entirely within U+00FF
    /// selects Latin-1. If the preference cannot represent the entire value,
    /// UTF-8 is used instead, never replacement text or mixed charsets.
    /// This is independent of a Content-Type charset describing the message body.
    pub parameter_charset: Charset,
    /// Allow raw UTF-8 in quoted parameter values, defaulting to false.
    ///
    /// Long values can use unencoded continuations with complete UTF-8 scalars.
    /// Controls and encoded-word lookalikes still use the selected encoding;
    /// RFC 2047 also remains the fallback for long values when selected.
    /// Raw values are UTF-8 regardless of the preferred parameter charset.
    /// This does not enable international addresses or negotiate transport
    /// support; callers must ensure the destination accepts UTF-8 headers.
    ///
    /// ```
    /// use mimecraft::{ContentDisposition, HeaderFormatOptions};
    ///
    /// let mut value = ContentDisposition::default();
    /// value.set_file_name(Some("caf\u{e9}.txt"));
    /// let options = HeaderFormatOptions { international: true, ..Default::default() };
    /// assert_eq!(value.to_header_string_with_options(&options)?,
    ///     "Content-Disposition: attachment; filename=\"caf\u{e9}.txt\"\r\n");
    /// # Ok::<(), mimecraft::HeaderFormatError>(())
    /// ```
    pub international: bool,
}

impl Default for HeaderFormatOptions {
    fn default() -> Self {
        Self {
            max_line_length: 78,
            new_line_format: NewLineFormat::Dos,
            always_quote_parameters: false,
            parameter_encoding_method: ParameterEncodingMethod::Rfc2231,
            parameter_charset: Charset::UTF8,
            international: false,
        }
    }
}

/// A complete MIME content header could not be serialized.
///
/// Formatting returns no partial string on error and does not mutate the header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HeaderFormatError {
    /// The requested line limit is outside `60..=998`.
    InvalidLineLength(InvalidLineLength),
    /// Mixed line endings cannot be emitted.
    InvalidNewLineFormat(NewLineFormat),
    /// An indivisible piece cannot fit on a continuation line.
    LineTooLong {
        /// Bytes needed, including indentation and any required separator.
        required: usize,
        /// The requested physical-line limit.
        limit: usize,
    },
    /// A parameter would need section numbers greater than `i32::MAX`.
    TooManyContinuations,
    /// Encoding a parameter as RFC 2047 encoded words failed.
    Encoding(rfc2047::EncodeError),
    /// A selected charset could not encode a complete RFC 2231 chunk.
    Charset(CharsetError),
}

impl fmt::Display for HeaderFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineLength(error) => error.fmt(f),
            Self::InvalidNewLineFormat(format) => {
                write!(
                    f,
                    "header new-line format must be Dos or Unix, got {format:?}"
                )
            }
            Self::LineTooLong { required, limit } => {
                write!(
                    f,
                    "header piece needs {required} bytes but the line limit is {limit}"
                )
            }
            Self::TooManyContinuations => f.write_str("parameter section number exceeds i32::MAX"),
            Self::Encoding(error) => error.fmt(f),
            Self::Charset(error) => error.fmt(f),
        }
    }
}

impl Error for HeaderFormatError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidLineLength(error) => Some(error),
            Self::Encoding(error) => Some(error),
            Self::Charset(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) fn format_header(
    field: &str,
    value: &str,
    parameters: &ParameterList,
    options: &HeaderFormatOptions,
) -> Result<String, HeaderFormatError> {
    let newline = match options.new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => {
            return Err(HeaderFormatError::InvalidNewLineFormat(
                NewLineFormat::Mixed,
            ));
        }
    };
    if !(60..=998).contains(&options.max_line_length) {
        return Err(HeaderFormatError::InvalidLineLength(
            InvalidLineLength::new(options.max_line_length),
        ));
    }
    let mut writer = HeaderWriter {
        output: format!("{field}:"),
        column: field.len() + 1,
        newline,
        options,
    };
    let reserve = usize::from(!parameters.is_empty());
    if writer.column + 1 + value.len() + reserve > options.max_line_length {
        writer.check_length(1 + value.len() + reserve)?;
        writer.fold();
    } else {
        writer.output.push(' ');
        writer.column += 1;
    }
    writer.output.push_str(value);
    writer.column += value.len();
    for (index, parameter) in parameters.iter().enumerate() {
        writer.parameter(parameter, index + 1 < parameters.len())?;
    }
    writer.output.push_str(newline);
    Ok(writer.output)
}

struct HeaderWriter<'a> {
    output: String,
    column: usize,
    newline: &'static str,
    options: &'a HeaderFormatOptions,
}

impl HeaderWriter<'_> {
    fn check_length(&self, required: usize) -> Result<(), HeaderFormatError> {
        if required > self.options.max_line_length {
            Err(HeaderFormatError::LineTooLong {
                required,
                limit: self.options.max_line_length,
            })
        } else {
            Ok(())
        }
    }

    fn fold(&mut self) {
        self.output.push_str(self.newline);
        self.output.push('\t');
        self.column = 1;
    }

    fn assignment(
        &mut self,
        name: &str,
        value: &str,
        force_fold: bool,
        more: bool,
    ) -> Result<(), HeaderFormatError> {
        let length = name.len() + 1 + value.len();
        let reserve = usize::from(more);
        self.check_length(1 + length + reserve)?;
        // Every preceding piece reserved a byte for this separator.
        self.output.push(';');
        self.column += 1;
        if force_fold || self.column + 1 + length + reserve >= self.options.max_line_length {
            self.fold();
        } else {
            self.output.push(' ');
            self.column += 1;
        }
        self.output.push_str(name);
        self.output.push('=');
        self.output.push_str(value);
        self.column += length;
        Ok(())
    }

    fn parameter(&mut self, parameter: &Parameter, more: bool) -> Result<(), HeaderFormatError> {
        let name = parameter.name();
        let value = parameter.value();
        let always_quote = self.options.always_quote_parameters || parameter.always_quote();
        // Quoting alone does not protect a literal encoded-word wrapper:
        // a tolerant parameter parser could decode it instead of retaining it.
        let extended = value.bytes().any(|byte| {
            byte.is_ascii_control() || (!byte.is_ascii() && !self.options.international)
        }) || value.contains("=?");
        if !extended {
            // Tokens cannot be empty. An empty value must be quoted and cannot
            // be split into continuations to make an oversized name fit.
            let quoted =
                always_quote || value.is_empty() || value.bytes().any(|byte| !is_attribute(byte));
            let length = if quoted {
                value.len()
                    + value
                        .bytes()
                        .filter(|&byte| matches!(byte, b'\\' | b'"'))
                        .count()
                    + 2
            } else {
                value.len()
            };
            if 1 + name.len() + 1 + length + usize::from(more) <= self.options.max_line_length
                || value.is_empty()
            {
                return self.assignment(
                    name,
                    &if quoted {
                        quote(value)
                    } else {
                        value.to_owned()
                    },
                    false,
                    more,
                );
            }
        }

        // Use one charset for the entire value. Choose it before
        // splitting so no later section loses unrepresentable characters.
        // An explicit charset is still a preference, not an override of
        // the automatic ASCII/Latin-1 selection.
        let charset = if value
            .bytes()
            .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
        {
            Charset::ASCII
        } else if value.chars().all(|ch| u32::from(ch) <= 255) {
            Charset::LATIN1
        } else {
            let preferred = parameter
                .charset()
                .unwrap_or(self.options.parameter_charset);
            // RFC 2231 allows only one charset for the entire value. Fall back
            // to UTF-8 if any character is unrepresentable instead of replacing
            // it with '?'. RFC 2047 also keeps one charset so re-encoding does
            // not drift when a reader remembers only the first word's charset.
            match preferred.encode(value) {
                Ok(_) => preferred,
                Err(CharsetError::UnmappableText(_)) => Charset::UTF8,
                Err(error) => return Err(HeaderFormatError::Charset(error)),
            }
        };
        if parameter
            .encoding_method()
            .unwrap_or(self.options.parameter_encoding_method)
            == ParameterEncodingMethod::Rfc2047
        {
            return self.encoded_words(name, value, charset, more);
        }
        let simple_charset =
            charset == Charset::ASCII || charset == Charset::LATIN1 || charset == Charset::UTF8;
        let mut chars = value.chars().peekable();
        let mut section = 0usize;
        while chars.peek().is_some() {
            if section > i32::MAX as usize {
                return Err(HeaderFormatError::TooManyContinuations);
            }
            let suffix = if extended { "*" } else { "" };
            let numbered_name = format!("{name}*{section}{suffix}");
            // The final scalar may use the separator's byte. All earlier
            // chunks reserve it, including when section numbers grow.
            let reserve = usize::from(more || chars.clone().nth(1).is_some());
            let overhead = 1 + numbered_name.len() + 1 + reserve;
            // Only section zero carries the charset declaration, even when the
            // character that requires extended encoding occurs much later.
            let prefix = if extended && section == 0 {
                format!("{}''", charset.name())
            } else {
                String::new()
            };
            let next = *chars.peek().expect("nonempty remainder");
            let minimum = if extended {
                prefix.len()
                    + if simple_charset {
                        encoded_length(next, charset)
                    } else {
                        let mut buffer = [0; 4];
                        let bytes = charset
                            .encode(next.encode_utf8(&mut buffer))
                            .map_err(HeaderFormatError::Charset)?;
                        percent_encoded_length(&bytes)
                    }
            } else {
                raw_length(next)
                    + if always_quote || !next.is_ascii() || !is_attribute(next as u8) {
                        2
                    } else {
                        0
                    }
            };
            // Keep MimeKit's conservative payload budget where possible, but
            // allow at least one complete scalar if the physical line permits.
            let preferred = self
                .options
                .max_line_length
                .saturating_sub(name.len() + 6)
                .max(minimum);
            let available = self.options.max_line_length.saturating_sub(overhead);
            let budget = preferred.min(available);
            let chunk = if extended && !simple_charset {
                next_charset_chunk(&mut chars, charset, &prefix, budget)
                    .map_err(HeaderFormatError::Charset)?
            } else {
                next_chunk(&mut chars, charset, extended, &prefix, budget, always_quote)
            }
            .ok_or(HeaderFormatError::LineTooLong {
                required: overhead + minimum,
                limit: self.options.max_line_length,
            })?;
            if section == 0 && chars.peek().is_none() {
                return self.assignment(&format!("{name}{suffix}"), &chunk, false, more);
            }
            self.assignment(&numbered_name, &chunk, true, chars.peek().is_some() || more)?;
            section += 1;
        }
        Ok(())
    }

    fn encoded_words(
        &mut self,
        name: &str,
        value: &str,
        charset: Charset,
        more: bool,
    ) -> Result<(), HeaderFormatError> {
        let reserve = usize::from(more);
        let overhead = 1 + name.len() + 3 + reserve;
        self.check_length(overhead + charset.name().len() + 8)?;
        // Use one representable charset for the entire value, not a mixture
        // that would change when parsed and re-encoded. Reserve the name,
        // surrounding quotes, indentation, and any following semicolon. Later
        // words can use the full continuation line without repeating the name.
        let words = rfc2047::encode_parameter(
            value,
            &rfc2047::EncodeOptions {
                charset,
                max_word_length: (self.options.max_line_length - 2 - reserve).min(75),
            },
            (self.options.max_line_length - overhead).min(75),
        )
        .map_err(|error| match error {
            rfc2047::EncodeError::WordTooLong {
                offset, required, ..
            } => HeaderFormatError::LineTooLong {
                required: if offset == 0 { overhead } else { 2 + reserve } + required,
                limit: self.options.max_line_length,
            },
            error => HeaderFormatError::Encoding(error),
        })?;
        let mut words = words.split(' ');
        let first = words.next().expect("nonempty encoded parameter");
        self.output.push(';');
        self.column += 1;
        if self.column + overhead + first.len() >= self.options.max_line_length {
            self.fold();
        } else {
            self.output.push(' ');
            self.column += 1;
        }
        self.output.push_str(name);
        self.output.push_str("=\"");
        self.output.push_str(first);
        self.column += name.len() + 2 + first.len();
        for word in words {
            if self.column + 1 + word.len() + 1 + reserve > self.options.max_line_length {
                self.fold();
            } else {
                self.output.push(' ');
                self.column += 1;
            }
            self.output.push_str(word);
            self.column += word.len();
        }
        self.output.push('"');
        self.column += 1;
        Ok(())
    }
}

fn character_bytes(ch: char, charset: Charset, buffer: &mut [u8; 4]) -> &[u8] {
    if charset == Charset::UTF8 {
        ch.encode_utf8(buffer).as_bytes()
    } else {
        // Automatic charset selection guarantees ASCII or Latin-1 representability.
        buffer[0] = u8::try_from(u32::from(ch)).expect("character fits the selected charset");
        &buffer[..1]
    }
}

fn encoded_length(ch: char, charset: Charset) -> usize {
    percent_encoded_length(character_bytes(ch, charset, &mut [0; 4]))
}

fn percent_encoded_length(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .map(|&byte| if is_attribute(byte) { 1 } else { 3 })
        .sum()
}

fn next_charset_chunk(
    chars: &mut Peekable<Chars<'_>>,
    charset: Charset,
    prefix: &str,
    budget: usize,
) -> Result<Option<String>, CharsetError> {
    if prefix.len() > budget {
        return Ok(None);
    }
    let mut text = String::new();
    let mut count = 0;
    for ch in chars.clone() {
        text.push(ch);
        // Never split a Unicode scalar (including a UTF-16 surrogate pair).
        // Convert each candidate as a complete chunk, accounting for any
        // state-reset bytes as well as percent escapes in the physical limit.
        // Encoding scalars separately would repeatedly shift ISO-2022 state.
        let bytes = charset.encode(&text)?;
        if prefix.len() + percent_encoded_length(&bytes) > budget {
            text.pop();
            break;
        }
        count += 1;
    }
    if count == 0 {
        // A charset-only first section can leave room for a complete character
        // on the next line. Later sections must consume at least one scalar.
        return Ok((!prefix.is_empty()).then(|| prefix.to_owned()));
    }
    let mut output = prefix.as_bytes().to_vec();
    Rfc2231Encoder::new().encode(&charset.encode(&text)?, &mut output);
    for _ in 0..count {
        chars.next();
    }
    Ok(Some(
        String::from_utf8(output).expect("percent output is ASCII"),
    ))
}

fn raw_length(ch: char) -> usize {
    if matches!(ch, '\\' | '"') {
        2
    } else {
        ch.len_utf8()
    }
}

fn next_chunk(
    chars: &mut Peekable<Chars<'_>>,
    charset: Charset,
    extended: bool,
    prefix: &str,
    budget: usize,
    always_quote: bool,
) -> Option<String> {
    if extended {
        if prefix.len() > budget {
            return None;
        }
        let mut bytes = prefix.as_bytes().to_vec();
        // Split at Unicode scalar boundaries, not encoded byte boundaries:
        // neither a multibyte character nor a percent triplet may be cut in two.
        while let Some(&ch) = chars.peek() {
            if bytes.len() + encoded_length(ch, charset) > budget {
                break;
            }
            Rfc2231Encoder::new().encode(character_bytes(ch, charset, &mut [0; 4]), &mut bytes);
            chars.next();
        }
        // A charset-only first section is useful when its declaration fits but
        // leaves no space for the first character. Later sections must progress.
        return (!bytes.is_empty())
            .then(|| String::from_utf8(bytes).expect("percent output is ASCII"));
    }
    let mut text = String::new();
    let mut escaped_length = 0;
    let mut quoted = always_quote;
    while let Some(&ch) = chars.peek() {
        // International sections stay UTF-8, independent of the charset used
        // for encoded sections. Never split a scalar's bytes between values.
        let needs_quotes = quoted || !ch.is_ascii() || !is_attribute(ch as u8);
        let length = escaped_length + raw_length(ch);
        if length + if needs_quotes { 2 } else { 0 } > budget {
            break;
        }
        text.push(ch);
        escaped_length = length;
        quoted = needs_quotes;
        chars.next();
    }
    if text.is_empty() {
        None
    } else if quoted {
        Some(quote(&text))
    } else {
        Some(text)
    }
}

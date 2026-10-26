// Adapted from MimeKit's ContentDisposition.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, str::FromStr};

use crate::{
    ContentHeaderParseError, ContentHeaderParseErrorKind, DateTimeOffset, DateTimeOffsetError,
    HeaderFormatError, HeaderFormatOptions, ParameterList, ParameterParseOptions,
    content_header::{parse_parameters, skip_cfws, validate_token},
    header_format::format_header,
    parse_utils::{is_ascii_atom, is_atom},
};

/// A decoded Content-Disposition header value with construction and editing support.
///
/// Disposition spelling is preserved. Unknown disposition atoms such as
/// `form-data` are accepted. Serialization normalizes formatting and parameter
/// encoding but does not sanitize filenames; preserve [`crate::Header::raw`]
/// separately for lossless output.
///
/// ```
/// use mimecraft::ContentDisposition;
///
/// let disposition = ContentDisposition::parse(
///     b"attachment; filename*=utf-8''caf%C3%A9.txt; size=123"
/// )?;
/// assert!(disposition.is_attachment());
/// assert_eq!(disposition.file_name(), Some("caf\u{e9}.txt"));
/// assert_eq!(disposition.size()?, Some(123));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentDisposition {
    disposition: String,
    parameters: ParameterList,
}

impl ContentDisposition {
    /// Creates a folded ASCII [`crate::Header`] with default formatting options.
    ///
    /// Uses [`Self::to_header_string`]'s encoding and CRLF termination, with
    /// the cached identifier [`crate::HeaderId::ContentDisposition`]. The typed
    /// value and any existing raw header remain unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] if an indivisible piece cannot fit.
    pub fn to_header(&self) -> Result<crate::Header, HeaderFormatError> {
        self.to_header_with_options(&HeaderFormatOptions::default())
    }

    /// Creates a Content-Disposition record with explicit formatting options.
    ///
    /// Folds and newline bytes match [`Self::to_header_string_with_options`].
    /// Construct the record before replacing a field with
    /// [`crate::HeaderList::replace_all`] so formatting errors leave the list intact.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] for invalid options or an oversized piece.
    pub fn to_header_with_options(
        &self,
        options: &HeaderFormatOptions,
    ) -> Result<crate::Header, HeaderFormatError> {
        self.to_header_string_with_options(options)
            .map(crate::Header::from_formatted)
    }

    /// Serializes a complete ASCII header, including its field name and a
    /// terminating CRLF, using default [`HeaderFormatOptions`].
    ///
    /// ```
    /// use mimecraft::ContentDisposition;
    ///
    /// let mut disposition = ContentDisposition::default();
    /// disposition.set_file_name(Some("caf\u{e9}.txt"));
    /// assert_eq!(
    ///     disposition.to_header_string()?,
    ///     "Content-Disposition: attachment; filename*=iso-8859-1''caf%E9.txt\r\n"
    /// );
    /// # Ok::<(), mimecraft::HeaderFormatError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] when an indivisible piece cannot fit.
    pub fn to_header_string(&self) -> Result<String, HeaderFormatError> {
        self.to_header_string_with_options(&HeaderFormatOptions::default())
    }

    /// Serializes a complete header using explicit folding and newline options.
    ///
    /// Output includes the field name and exactly one terminating newline.
    /// Unicode, controls, and encoded-word lookalikes use the selected encoding,
    /// preserving their decoded values without writing raw header delimiters.
    /// The value, diagnostics, and any separate raw header remain unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] for invalid options or an unbreakable piece
    /// that cannot fit within the requested physical-line limit.
    pub fn to_header_string_with_options(
        &self,
        options: &HeaderFormatOptions,
    ) -> Result<String, HeaderFormatError> {
        format_header(
            "Content-Disposition",
            &self.disposition,
            &self.parameters,
            options,
        )
    }

    /// The attachment disposition token.
    pub const ATTACHMENT: &str = "attachment";
    /// The inline disposition token.
    pub const INLINE: &str = "inline";

    /// Creates a disposition with no parameters.
    ///
    /// Accepts any nonempty ASCII atom, not just `attachment` and `inline`.
    /// Comments, surrounding whitespace, and quoted strings are not accepted.
    /// [`Self::default`] creates an attachment disposition.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDisposition` at the first invalid byte in `disposition`,
    /// or byte zero for an empty value.
    pub fn new(disposition: &str) -> Result<Self, ContentHeaderParseError> {
        validate_token(
            disposition,
            is_ascii_atom,
            ContentHeaderParseErrorKind::InvalidDisposition,
        )?;
        Ok(Self {
            disposition: disposition.to_owned(),
            parameters: ParameterList::new(),
        })
    }

    /// Parses a complete header value with default parameter options.
    ///
    /// Supply the value only, not the `Content-Disposition:` field name. Like
    /// MimeKit's throwing `Parse` path, this rejects quoted or missing disposition
    /// atoms; it does not apply the attachment fallback of `TryParse`.
    /// Non-ASCII disposition bytes are rejected rather than silently replaced
    /// with question marks by the source's ASCII conversion.
    ///
    /// # Errors
    ///
    /// Returns a syntax error for an invalid disposition, missing parameter
    /// separator, incomplete comment, or invalid parameter list.
    pub fn parse(input: &[u8]) -> Result<Self, ContentHeaderParseError> {
        Self::parse_with_options(input, &ParameterParseOptions::default())
    }

    /// Parses with explicit parameter syntax, encoded-word, and charset options.
    ///
    /// These options affect parameters only. Parameter diagnostic ranges and
    /// all error offsets are relative to this complete input slice. Typed date
    /// and size parameters are interpreted lazily by their respective accessors.
    ///
    /// # Errors
    ///
    /// Returns the syntax failures described by [`Self::parse`].
    pub fn parse_with_options(
        input: &[u8],
        options: &ParameterParseOptions,
    ) -> Result<Self, ContentHeaderParseError> {
        let mut index = 0;
        skip_cfws(input, &mut index)?;
        let start = index;
        while let Some(&byte) = input.get(index).filter(|&&byte| is_atom(byte)) {
            if !byte.is_ascii() {
                return Err(ContentHeaderParseError::new(
                    ContentHeaderParseErrorKind::InvalidDisposition,
                    index,
                ));
            }
            index += 1;
        }
        if index == start {
            return Err(ContentHeaderParseError::new(
                ContentHeaderParseErrorKind::InvalidDisposition,
                start,
            ));
        }
        let disposition = std::str::from_utf8(&input[start..index])
            .expect("disposition is ASCII")
            .to_owned();
        let parameters = parse_parameters(input, index, options)?;
        Ok(Self {
            disposition,
            parameters,
        })
    }

    /// Returns the original spelling of the disposition atom.
    pub fn disposition(&self) -> &str {
        &self.disposition
    }

    /// Replaces the disposition, preserving parameters and historical diagnostics.
    ///
    /// # Errors
    ///
    /// Applies [`Self::new`]'s atom rules. Error offsets refer to `value`;
    /// invalid values leave this disposition unchanged.
    pub fn set_disposition(&mut self, value: &str) -> Result<(), ContentHeaderParseError> {
        validate_token(
            value,
            is_ascii_atom,
            ContentHeaderParseErrorKind::InvalidDisposition,
        )?;
        self.disposition = value.to_owned();
        Ok(())
    }

    /// Returns whether the disposition is `attachment`, ignoring ASCII case.
    pub fn is_attachment(&self) -> bool {
        self.disposition.eq_ignore_ascii_case(Self::ATTACHMENT)
    }

    /// Sets `attachment` for `true`, or `inline` for `false`, preserving parameters.
    pub fn set_is_attachment(&mut self, value: bool) {
        self.disposition = if value {
            Self::ATTACHMENT
        } else {
            Self::INLINE
        }
        .to_owned();
    }

    /// Returns decoded parameters and diagnostics relative to the full input.
    pub fn parameters(&self) -> &ParameterList {
        &self.parameters
    }

    /// Allows ordered parameter edits while preserving case-insensitive uniqueness.
    ///
    /// Diagnostics remain a record of original input, even after edits. No raw
    /// header bytes are updated.
    pub fn parameters_mut(&mut self) -> &mut ParameterList {
        &mut self.parameters
    }

    /// Returns the decoded filename without stripping paths or control characters.
    pub fn file_name(&self) -> Option<&str> {
        self.parameters.get("filename")
    }

    /// Sets the decoded filename literally, or removes it for `None`.
    ///
    /// Quotes, Unicode, controls, and paths are not interpreted or sanitized.
    pub fn set_file_name(&mut self, value: Option<&str>) {
        self.parameters
            .set_optional("filename", value.map(str::to_owned));
    }

    /// Interprets the creation date using [`DateTimeOffset::parse`].
    ///
    /// # Errors
    ///
    /// Returns a date error for an uninterpretable value, including an empty
    /// value. Only an absent parameter returns `Ok(None)`.
    pub fn creation_date(&self) -> Result<Option<DateTimeOffset>, DateTimeOffsetError> {
        self.date_parameter("creation-date")
    }

    /// Sets a canonically formatted creation date, or removes it for `None`.
    pub fn set_creation_date(&mut self, value: Option<DateTimeOffset>) {
        self.set_date_parameter("creation-date", value);
    }

    /// Interprets the modification date using [`DateTimeOffset::parse`].
    ///
    /// # Errors
    ///
    /// Returns a date error for an uninterpretable or empty value; absence alone
    /// returns `Ok(None)`.
    pub fn modification_date(&self) -> Result<Option<DateTimeOffset>, DateTimeOffsetError> {
        self.date_parameter("modification-date")
    }

    /// Sets a canonically formatted modification date, or removes it for `None`.
    pub fn set_modification_date(&mut self, value: Option<DateTimeOffset>) {
        self.set_date_parameter("modification-date", value);
    }

    /// Interprets the read date using [`DateTimeOffset::parse`].
    ///
    /// # Errors
    ///
    /// Returns a date error for an uninterpretable or empty value; absence alone
    /// returns `Ok(None)`.
    pub fn read_date(&self) -> Result<Option<DateTimeOffset>, DateTimeOffsetError> {
        self.date_parameter("read-date")
    }

    /// Sets a canonically formatted read date, or removes it for `None`.
    pub fn set_read_date(&mut self, value: Option<DateTimeOffset>) {
        self.set_date_parameter("read-date", value);
    }

    fn date_parameter(&self, name: &str) -> Result<Option<DateTimeOffset>, DateTimeOffsetError> {
        self.parameters
            .get(name)
            .map(|value| DateTimeOffset::parse(value.as_bytes()))
            .transpose()
    }

    fn set_date_parameter(&mut self, name: &str, value: Option<DateTimeOffset>) {
        self.parameters
            .set_optional(name, value.map(|date| date.to_string()));
    }

    /// Interprets the decoded size parameter as nonnegative decimal bytes.
    ///
    /// Only ASCII digits are accepted, with no sign or surrounding whitespace.
    /// Values are limited to `i64::MAX` for MimeKit compatibility, despite the
    /// unsigned return type. This is sender-supplied metadata, not a measured size.
    ///
    /// # Errors
    ///
    /// Returns [`ContentDispositionSizeError`] for invalid digits, an empty
    /// value, or overflow. Only an absent parameter returns `Ok(None)`.
    pub fn size(&self) -> Result<Option<u64>, ContentDispositionSizeError> {
        let Some(value) = self.parameters.get("size") else {
            return Ok(None);
        };
        if value.is_empty() {
            return Err(ContentDispositionSizeError::InvalidDigits);
        }
        let mut size = 0u64;
        for byte in value.bytes() {
            if !byte.is_ascii_digit() {
                return Err(ContentDispositionSizeError::InvalidDigits);
            }
            let digit = u64::from(byte - b'0');
            if size > (i64::MAX as u64 - digit) / 10 {
                return Err(ContentDispositionSizeError::Overflow);
            }
            size = size * 10 + digit;
        }
        Ok(Some(size))
    }

    /// Sets the declared byte size, or removes it for `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ContentDispositionSizeError::Overflow`] for values greater
    /// than `i64::MAX`, leaving the previous parameter unchanged.
    pub fn set_size(&mut self, value: Option<u64>) -> Result<(), ContentDispositionSizeError> {
        if value.is_some_and(|size| size > i64::MAX as u64) {
            return Err(ContentDispositionSizeError::Overflow);
        }
        self.parameters
            .set_optional("size", value.map(|size| size.to_string()));
        Ok(())
    }
}

impl Default for ContentDisposition {
    fn default() -> Self {
        Self {
            disposition: Self::ATTACHMENT.to_owned(),
            parameters: ParameterList::new(),
        }
    }
}

impl FromStr for ContentDisposition {
    type Err = ContentHeaderParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.as_bytes())
    }
}

/// A Content-Disposition size value could not be interpreted or assigned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentDispositionSizeError {
    /// The value is empty or contains characters other than ASCII digits.
    InvalidDigits,
    /// The value exceeds MimeKit's signed 64-bit maximum.
    Overflow,
}

impl fmt::Display for ContentDispositionSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDigits => "size must contain one or more ASCII digits",
            Self::Overflow => "size exceeds i64::MAX",
        })
    }
}

impl Error for ContentDispositionSizeError {}

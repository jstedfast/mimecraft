// Adapted from MimeKit's ContentType.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::str::FromStr;

use crate::{
    ContentHeaderParseError, ContentHeaderParseErrorKind, HeaderFormatError, HeaderFormatOptions,
    ParameterList, ParameterParseOptions,
    charsets::{Charset, CharsetError},
    content_header::{parse_parameters, skip_cfws, validate_token},
    header_format::format_header,
    parse_utils::{is_media_subtype, is_media_type},
};

/// A decoded Content-Type header value with construction and editing support.
///
/// Type/subtype spelling is preserved; use [`Self::is_mime_type`] for
/// case-insensitive comparisons. Serialization normalizes formatting and
/// parameter encoding. Preserve [`crate::Header::raw`] separately for lossless output.
///
/// ```
/// use mimecraft::ContentType;
///
/// let content_type = ContentType::parse(
///     b"Text/Plain; charset=utf-8; format=flowed"
/// )?;
/// assert!(content_type.is_mime_type("text", "*"));
/// assert_eq!(content_type.charset(), Some("utf-8"));
/// assert_eq!(content_type.format(), Some("flowed"));
/// # Ok::<(), mimecraft::ContentHeaderParseError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentType {
    media_type: String,
    media_subtype: String,
    parameters: ParameterList,
}

impl ContentType {
    /// Creates a folded ASCII [`crate::Header`] with default formatting options.
    ///
    /// Uses [`Self::to_header_string`]'s encoding and CRLF termination, with
    /// the cached identifier [`crate::HeaderId::ContentType`]. The typed value
    /// and any existing raw header remain unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] if an indivisible piece cannot fit.
    pub fn to_header(&self) -> Result<crate::Header, HeaderFormatError> {
        self.to_header_with_options(&HeaderFormatOptions::default())
    }

    /// Creates a Content-Type record with explicit formatting options.
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

    /// Serializes a complete ASCII header, including `Content-Type: ` and a
    /// terminating CRLF, using default [`HeaderFormatOptions`].
    ///
    /// Parameter values use quoting or RFC 2231 encoding as needed, unless a
    /// parameter explicitly selects RFC 2047 compatibility output. This does not
    /// mutate the value, its diagnostics, or a separately stored raw header.
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
    /// [`HeaderFormatOptions::international`] allows raw UTF-8 parameters.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] for invalid options or an unbreakable piece
    /// that cannot fit within the requested physical-line limit.
    pub fn to_header_string_with_options(
        &self,
        options: &HeaderFormatOptions,
    ) -> Result<String, HeaderFormatError> {
        format_header("Content-Type", &self.mime_type(), &self.parameters, options)
    }

    /// Creates a content type with an empty parameter list.
    ///
    /// Components must be nonempty and use the same ASCII alphabets as this
    /// type's parser, without comments or surrounding whitespace. This includes
    /// MimeKit's extra-slash subtype compatibility; it is not strict RFC
    /// validation. Unlike MimeKit's unchecked constructor, malformed components
    /// cannot be assigned.
    ///
    /// ```
    /// use mimecraft::ContentType;
    ///
    /// let mut content_type = ContentType::new("text", "plain")?;
    /// content_type.set_charset(Some("utf-8"));
    /// content_type.set_format(Some("flowed"));
    /// assert_eq!(content_type.charset(), Some("utf-8"));
    /// # Ok::<(), mimecraft::ContentHeaderParseError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `InvalidMediaType` or `InvalidMediaSubtype`, with the first invalid
    /// byte offset in that component (zero for an empty component).
    pub fn new(media_type: &str, media_subtype: &str) -> Result<Self, ContentHeaderParseError> {
        validate_token(
            media_type,
            is_media_type,
            ContentHeaderParseErrorKind::InvalidMediaType,
        )?;
        validate_token(
            media_subtype,
            is_media_subtype,
            ContentHeaderParseErrorKind::InvalidMediaSubtype,
        )?;
        Ok(Self {
            media_type: media_type.to_owned(),
            media_subtype: media_subtype.to_owned(),
            parameters: ParameterList::new(),
        })
    }

    /// Parses a complete header value with default parameter options.
    ///
    /// Supply the value only, not the `Content-Type:` field name. Comments and
    /// whitespace are accepted around tokens and the slash. For compatibility
    /// with MimeKit, subtypes may contain extra slashes, such as
    /// `multipart/multipart/mixed`. Parsing is not a strict MIME validator.
    ///
    /// # Errors
    ///
    /// Returns a syntax error for a missing/invalid type, slash, or subtype,
    /// missing parameter separator, incomplete comment, or invalid parameters.
    pub fn parse(input: &[u8]) -> Result<Self, ContentHeaderParseError> {
        Self::parse_with_options(input, &ParameterParseOptions::default())
    }

    /// Parses with explicit parameter syntax, encoded-word, and charset options.
    ///
    /// These options affect parameters only. Parameter diagnostic ranges and
    /// all error offsets are relative to this complete input slice.
    ///
    /// # Errors
    ///
    /// Returns the syntax failures described by [`Self::parse`].
    pub fn parse_with_options(
        input: &[u8],
        options: &ParameterParseOptions,
    ) -> Result<Self, ContentHeaderParseError> {
        let (mut content_type, index) = Self::parse_mime_type(input)?;
        content_type.parameters = parse_parameters(input, index, options)?;
        Ok(content_type)
    }

    // MimeReader keeps a recognizable MIME type even when its parameters
    // cannot be parsed. An unintelligible type becomes application/octet-stream,
    // with a separate attempt to recover its parameter list. The original raw
    // header remains intact; the boolean requests an InvalidContentType report.
    pub(crate) fn parse_for_reader(input: &[u8]) -> (Self, bool) {
        let (mut content_type, index, invalid) = match Self::parse_mime_type(input) {
            Ok((content_type, index)) => (content_type, index, false),
            Err(error) => {
                let index = input[error.offset()..]
                    .iter()
                    .position(|&byte| byte == b';')
                    .map_or(input.len(), |index| error.offset() + index);
                (
                    Self::new("application", "octet-stream").expect("constant MIME type"),
                    index,
                    true,
                )
            }
        };
        if let Ok(parameters) = parse_parameters(input, index, &ParameterParseOptions::default()) {
            content_type.parameters = parameters;
        }
        (content_type, invalid)
    }

    fn parse_mime_type(input: &[u8]) -> Result<(Self, usize), ContentHeaderParseError> {
        let mut index = 0;
        skip_cfws(input, &mut index)?;
        let start = index;
        while input.get(index).is_some_and(|&byte| is_media_type(byte)) {
            index += 1;
        }
        if index == start {
            return Err(ContentHeaderParseError::new(
                ContentHeaderParseErrorKind::InvalidMediaType,
                start,
            ));
        }
        let media_type = std::str::from_utf8(&input[start..index])
            .expect("media type is ASCII")
            .to_owned();
        skip_cfws(input, &mut index)?;
        if input.get(index) != Some(&b'/') {
            return Err(ContentHeaderParseError::new(
                ContentHeaderParseErrorKind::ExpectedSlash,
                index,
            ));
        }
        index += 1;
        skip_cfws(input, &mut index)?;
        let start = index;
        while input.get(index).is_some_and(|&byte| is_media_subtype(byte)) {
            index += 1;
        }
        if index == start {
            return Err(ContentHeaderParseError::new(
                ContentHeaderParseErrorKind::InvalidMediaSubtype,
                start,
            ));
        }
        let media_subtype = std::str::from_utf8(&input[start..index])
            .expect("media subtype is ASCII")
            .to_owned();
        skip_cfws(input, &mut index)?;
        Ok((
            Self {
                media_type,
                media_subtype,
                parameters: ParameterList::default(),
            },
            index,
        ))
    }

    /// Returns the original spelling of the media type.
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Replaces the media type, preserving subtype, parameters, and diagnostics.
    ///
    /// # Errors
    ///
    /// Applies [`Self::new`]'s component rules. The error offset refers to `value`;
    /// an invalid value leaves the content type unchanged.
    pub fn set_media_type(&mut self, value: &str) -> Result<(), ContentHeaderParseError> {
        validate_token(
            value,
            is_media_type,
            ContentHeaderParseErrorKind::InvalidMediaType,
        )?;
        self.media_type = value.to_owned();
        Ok(())
    }

    /// Returns the original spelling of the media subtype.
    pub fn media_subtype(&self) -> &str {
        &self.media_subtype
    }

    /// Replaces the media subtype, preserving type, parameters, and diagnostics.
    ///
    /// # Errors
    ///
    /// Applies [`Self::new`]'s component rules. The error offset refers to `value`;
    /// an invalid value leaves the content type unchanged.
    pub fn set_media_subtype(&mut self, value: &str) -> Result<(), ContentHeaderParseError> {
        validate_token(
            value,
            is_media_subtype,
            ContentHeaderParseErrorKind::InvalidMediaSubtype,
        )?;
        self.media_subtype = value.to_owned();
        Ok(())
    }

    /// Returns `type/subtype`, preserving case and excluding parameters.
    ///
    /// This is not a serialized Content-Type header value.
    pub fn mime_type(&self) -> String {
        format!("{}/{}", self.media_type, self.media_subtype)
    }

    /// Compares type and subtype case-insensitively.
    ///
    /// An argument equal to `"*"` matches any corresponding component; embedded
    /// wildcards such as `"*+xml"` have no special meaning.
    pub fn is_mime_type(&self, media_type: &str, media_subtype: &str) -> bool {
        (media_type == "*" || self.media_type.eq_ignore_ascii_case(media_type))
            && (media_subtype == "*" || self.media_subtype.eq_ignore_ascii_case(media_subtype))
    }

    /// Returns decoded parameters and diagnostics relative to the full input.
    pub fn parameters(&self) -> &ParameterList {
        &self.parameters
    }

    /// Allows ordered parameter edits while preserving case-insensitive uniqueness.
    ///
    /// Diagnostics continue to describe the original parsed input, not the
    /// current edited value. No raw header bytes are updated.
    pub fn parameters_mut(&mut self) -> &mut ParameterList {
        &mut self.parameters
    }

    /// Returns the decoded boundary without validating it as a multipart delimiter.
    pub fn boundary(&self) -> Option<&str> {
        self.parameters.get("boundary")
    }

    /// Sets the literal decoded boundary, or removes it for `None`.
    ///
    /// This does not validate boundary syntax or alter any multipart body.
    pub fn set_boundary(&mut self, value: Option<&str>) {
        self.parameters
            .set_optional("boundary", value.map(str::to_owned));
    }

    /// Returns the declared charset label without resolving it.
    pub fn charset(&self) -> Option<&str> {
        self.parameters.get("charset")
    }

    /// Sets the literal charset label without resolving it, or removes it for `None`.
    pub fn set_charset(&mut self, value: Option<&str>) {
        self.parameters
            .set_optional("charset", value.map(str::to_owned));
    }

    /// Resolves the charset parameter; absence is `Ok(None)`.
    ///
    /// # Errors
    ///
    /// Returns a lookup error for an unsupported label, including an empty value,
    /// rather than treating it as absent as MimeKit's convenience getter does.
    pub fn charset_encoding(&self) -> Result<Option<Charset>, CharsetError> {
        self.charset().map(Charset::from_label).transpose()
    }

    /// Sets a charset's canonical label, or removes the parameter for `None`.
    pub fn set_charset_encoding(&mut self, value: Option<Charset>) {
        self.set_charset(value.map(|charset| charset.name()));
    }

    /// Returns the decoded format parameter, typically `fixed` or `flowed`.
    pub fn format(&self) -> Option<&str> {
        self.parameters.get("format")
    }

    /// Sets the literal format parameter, or removes it for `None`.
    ///
    /// Values are not limited to `fixed` and `flowed`.
    pub fn set_format(&mut self, value: Option<&str>) {
        self.parameters
            .set_optional("format", value.map(str::to_owned));
    }

    /// Returns the decoded display-name parameter without path sanitization.
    pub fn name(&self) -> Option<&str> {
        self.parameters.get("name")
    }

    /// Sets a decoded display name without sanitization, or removes it for `None`.
    pub fn set_name(&mut self, value: Option<&str>) {
        self.parameters
            .set_optional("name", value.map(str::to_owned));
    }
}

impl FromStr for ContentType {
    type Err = ContentHeaderParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.as_bytes())
    }
}

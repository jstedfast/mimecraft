// Adapted from MimeKit's ContentType.cs and ContentDisposition.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::{
    ParameterList, ParameterParseError, ParameterParseErrorKind, ParameterParseOptions,
    parse_utils::skip_comments_and_whitespace,
};

/// The reason a Content-Type or Content-Disposition value could not be parsed or assigned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentHeaderParseErrorKind {
    /// An ASCII media type atom was expected.
    InvalidMediaType,
    /// The slash separating media type and subtype was missing.
    ExpectedSlash,
    /// A media subtype token was expected.
    InvalidMediaSubtype,
    /// An unquoted ASCII disposition atom was expected.
    InvalidDisposition,
    /// A semicolon was required before the parameter list.
    ExpectedSeparator,
    /// A comment did not have a closing parenthesis.
    IncompleteComment,
    /// The parameter list contained invalid syntax.
    Parameter(ParameterParseErrorKind),
}

/// A typed MIME header syntax failure with an original-input byte offset.
///
/// When parsing, offsets include the media type or disposition and any leading
/// whitespace. Constructors and token setters instead report offsets in the
/// affected component argument, identified by the error kind.
/// No partial header value is returned on failure; failed edits do not mutate it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentHeaderParseError {
    kind: ContentHeaderParseErrorKind,
    offset: usize,
}

impl ContentHeaderParseError {
    pub(crate) fn new(kind: ContentHeaderParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// Returns the reason parsing failed, including any parameter error kind.
    pub fn kind(self) -> ContentHeaderParseErrorKind {
        self.kind
    }

    /// Returns the failure's byte offset, possibly equal to the input length.
    ///
    /// Unterminated comments and quoted parameter values point to their opener.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl From<ParameterParseError> for ContentHeaderParseError {
    fn from(error: ParameterParseError) -> Self {
        Self::new(
            ContentHeaderParseErrorKind::Parameter(error.kind()),
            error.offset(),
        )
    }
}

impl fmt::Display for ContentHeaderParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            ContentHeaderParseErrorKind::InvalidMediaType => "expected an ASCII media type atom",
            ContentHeaderParseErrorKind::ExpectedSlash => "expected '/' before the media subtype",
            ContentHeaderParseErrorKind::InvalidMediaSubtype => "expected a media subtype token",
            ContentHeaderParseErrorKind::InvalidDisposition => "expected an ASCII disposition atom",
            ContentHeaderParseErrorKind::ExpectedSeparator => "expected ';' before MIME parameters",
            ContentHeaderParseErrorKind::IncompleteComment => "unterminated comment",
            ContentHeaderParseErrorKind::Parameter(kind) => {
                return ParameterParseError::new(kind, self.offset).fmt(f);
            }
        };
        write!(f, "{reason} at byte {}", self.offset)
    }
}

impl Error for ContentHeaderParseError {}

pub(crate) fn validate_token(
    value: &str,
    valid: fn(u8) -> bool,
    kind: ContentHeaderParseErrorKind,
) -> Result<(), ContentHeaderParseError> {
    let invalid = if value.is_empty() {
        Some(0)
    } else {
        value.bytes().position(|byte| !valid(byte))
    };
    if let Some(offset) = invalid {
        return Err(ContentHeaderParseError::new(kind, offset));
    }
    Ok(())
}

pub(crate) fn skip_cfws(input: &[u8], index: &mut usize) -> Result<(), ContentHeaderParseError> {
    skip_comments_and_whitespace(input, index).map_err(|offset| {
        ContentHeaderParseError::new(ContentHeaderParseErrorKind::IncompleteComment, offset)
    })
}

pub(crate) fn parse_parameters(
    input: &[u8],
    mut index: usize,
    options: &ParameterParseOptions,
) -> Result<ParameterList, ContentHeaderParseError> {
    skip_cfws(input, &mut index)?;
    if index == input.len() {
        return Ok(ParameterList::default());
    }
    if input[index] != b';' {
        return Err(ContentHeaderParseError::new(
            ContentHeaderParseErrorKind::ExpectedSeparator,
            index,
        ));
    }
    ParameterList::parse_from(input, index + 1, options).map_err(Into::into)
}

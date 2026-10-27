// Adapted from MimeKit's Parameter.cs, ParameterList.cs and Utils/ByteExtensions.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    collections::HashMap,
    error::Error,
    fmt,
    hash::{Hash, Hasher},
    ops::Range,
    str::FromStr,
};

use crate::{
    charsets::{Charset, decode_best},
    encodings::Rfc2231Decoder,
    parse_utils::{
        is_attribute, is_token, is_whitespace, skip_comments_and_whitespace, skip_quoted,
    },
    rfc2047::{self, DecodeIssue},
    utils::{unquote, unquote_bytes},
};

/// A decoded Content-Type or Content-Disposition parameter.
///
/// Names cannot be changed in place. Parsed names retain their first encountered
/// spelling with continuation suffixes removed. Values are Unicode strings,
/// not raw header syntax. Authored parameters may override the output encoding
/// method, prefer a charset, and request quoting. Parsing recovers encoding
/// preferences from extended parameters and recognized encoded words, but not
/// language tags or a request to retain unnecessary quotes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    name: String,
    value: String,
    encoding_method: Option<ParameterEncodingMethod>,
    charset: Option<Charset>,
    always_quote: bool,
}

/// The encoding used when a MIME parameter cannot be emitted as a short value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ParameterEncodingMethod {
    /// RFC 2231 extended parameters and continuations (the default).
    #[default]
    Rfc2231,
    /// RFC 2047 encoded words inside a quoted value, for compatibility.
    ///
    /// This is a legacy mail-client convention, not standards-compliant
    /// parameter encoding. Prefer RFC 2231 unless compatibility requires it.
    Rfc2047,
}

impl Parameter {
    /// Creates a parameter from a name and a decoded Unicode value.
    ///
    /// The name must be nonempty ASCII attribute characters, without RFC 2231
    /// suffixes. The value is stored literally, including quotes, encoded-word
    /// lookalikes, line breaks and NUL; it is not safe to insert directly into a
    /// header. Use [`crate::ContentType::to_header_string`] or
    /// [`crate::ContentDisposition::to_header_string`] to encode a containing header.
    ///
    /// # Errors
    ///
    /// Returns [`ParameterParseErrorKind::InvalidName`] at the first invalid
    /// byte in `name`, or byte zero for an empty name.
    pub fn new(name: &str, value: impl Into<String>) -> Result<Self, ParameterParseError> {
        validate_name(name)?;
        Ok(Self {
            name: name.to_owned(),
            value: value.into(),
            encoding_method: None,
            charset: None,
            always_quote: false,
        })
    }

    /// Returns the parameter name, without RFC 2231 suffixes.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the decoded value, without syntactic quotes or charset prefixes.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Replaces the decoded value literally, without parsing or sanitizing it.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
    }

    /// Returns the output override, or `None` to inherit header formatting options.
    pub fn encoding_method(&self) -> Option<ParameterEncodingMethod> {
        self.encoding_method
    }

    /// Selects the output encoding method, or restores inheritance with `None`.
    ///
    /// Short ASCII values still use ordinary tokens or quoted strings. Encoding
    /// is needed for controls, non-ASCII text, encoded-word lookalikes, and values
    /// too long for an ordinary assignment.
    /// [`crate::HeaderFormatOptions::international`] allows safe non-ASCII values
    /// to use ordinary quoted strings or unencoded RFC 2231 continuations.
    ///
    /// ```
    /// use mimecraft::{ContentDisposition, Parameter, ParameterEncodingMethod};
    ///
    /// let mut parameter = Parameter::new("filename", "caf\u{e9}.txt")?;
    /// parameter.set_encoding_method(Some(ParameterEncodingMethod::Rfc2047));
    /// let mut disposition = ContentDisposition::default();
    /// disposition.parameters_mut().add(parameter)?;
    /// assert!(disposition.to_header_string()?.contains("filename=\"=?"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_encoding_method(&mut self, method: Option<ParameterEncodingMethod>) {
        self.encoding_method = method;
    }

    /// Returns the preferred output charset, or `None` to inherit header options.
    ///
    /// Parsing retains the first extended section's charset (including Latin-1
    /// recovery), or the most common recognized encoded-word charset. Ties use
    /// first encounter order. RFC 2047 UTF-8 uses inheritance, as in MimeKit;
    /// ordinary text and a predominant unsupported label also use inheritance.
    /// This is a formatting preference, not an exact record of the original label.
    ///
    /// ```
    /// use mimecraft::{ParameterList, ParameterEncodingMethod, charsets::Charset};
    ///
    /// let parameters = ParameterList::parse(b"name*=ISO_8859_1'fr'caf%E9")?;
    /// let parameter = &parameters.as_slice()[0];
    /// assert_eq!(parameter.charset(), Some(Charset::LATIN1));
    /// assert_eq!(parameter.encoding_method(), Some(ParameterEncodingMethod::Rfc2231));
    /// # Ok::<(), mimecraft::ParameterParseError>(())
    /// ```
    pub fn charset(&self) -> Option<Charset> {
        self.charset
    }

    /// Sets the preferred output charset, or restores inheritance with `None`.
    ///
    /// Like MimeKit, formatting still selects ASCII for printable ASCII and
    /// Latin-1 for controls and text entirely within U+00FF. Other text uses this
    /// preference if representable, or UTF-8 for the entire value otherwise.
    /// This never enables lossy substitution or forces a charset on plain tokens.
    /// Raw international parameter output is always UTF-8, ignoring this preference.
    /// Mapping limitations are those of [`Charset::encode`].
    ///
    /// ```
    /// use mimecraft::{ContentDisposition, Parameter, charsets::Charset};
    ///
    /// let mut parameter = Parameter::new("filename", "\u{6d4b}\u{8bd5}.txt")?;
    /// parameter.set_charset(Some(Charset::from_label("gb18030")?));
    /// let mut disposition = ContentDisposition::default();
    /// disposition.parameters_mut().add(parameter)?;
    /// assert!(disposition.to_header_string()?.contains("gb18030''"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_charset(&mut self, charset: Option<Charset>) {
        self.charset = charset;
    }

    /// Returns whether this parameter requests quoting, initially false.
    pub fn always_quote(&self) -> bool {
        self.always_quote
    }

    /// Requests quoting of ordinary values and unencoded continuation sections.
    ///
    /// False does not override a header-wide quoting request. Extended RFC 2231
    /// sections remain percent-encoded, while RFC 2047 output is always quoted.
    pub fn set_always_quote(&mut self, value: bool) {
        self.always_quote = value;
    }
}

/// Parameter syntax recognition, independent of encoded-word recognition.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ParameterComplianceMode {
    /// Accept unquoted spaces/folding and missing semicolons after quoted values.
    #[default]
    Loose,
    /// Require token or quoted-string values and semicolon separators.
    ///
    /// Continuation recovery and tolerant payload decoding still apply.
    Strict,
}

/// Options for parsing a complete MIME parameter list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParameterParseOptions {
    /// Parameter syntax recognition, defaulting to loose mode.
    pub compliance_mode: ParameterComplianceMode,
    /// Encoded-word recognition in ordinary, non-continuation values.
    pub rfc2047_compliance_mode: rfc2047::ComplianceMode,
    /// Preferred charset for unlabelled text and unknown RFC 2047 labels.
    ///
    /// UTF-8 takes priority and Latin-1 is the final fallback. RFC 2231 values
    /// with missing or unsupported declarations use Latin-1 instead, as in MimeKit.
    pub fallback_charset: Option<Charset>,
}

/// An ordered, case-insensitively named collection of decoded MIME parameters.
///
/// Parse only the parameter portion, with or without its leading semicolon,
/// not the media type or disposition token. Continuations are sorted numerically
/// and joined, including out-of-order, missing, and duplicate section numbers.
/// Extended/continued definitions override ordinary duplicates; otherwise the
/// first ordinary or last extended definition wins. Original order and spelling
/// are retained at the first occurrence of each name.
///
/// Edits preserve case-insensitive uniqueness. [`Self::set`] keeps an existing
/// name's spelling and position; [`Self::replace`] can change a name explicitly.
/// Parse diagnostics remain a historical record of the original input even
/// after editing or clearing the list, not validation of its current contents.
/// Serialization is not implemented. Use [`crate::Header::raw`] to retain
/// original bytes. Values are not sanitized for paths, NUL, or controls.
///
/// ```
/// use mimecraft::ParameterList;
///
/// let parameters = ParameterList::parse(
///     b"; filename*1*=%A9.txt; filename*0*=utf-8''caf%C3"
/// )?;
/// assert_eq!(parameters.get("FILENAME"), Some("caf\u{e9}.txt"));
/// assert!(parameters.issues().is_empty());
/// # Ok::<(), mimecraft::ParameterParseError>(())
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[must_use]
pub struct ParameterList {
    parameters: Vec<Parameter>,
    issues: Vec<ParameterIssue>,
}

impl ParameterList {
    /// Creates an empty parameter list with no parse diagnostics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a decoded value, returning the old value if the name already existed.
    ///
    /// Comparison is ASCII case-insensitive. Existing spelling and position are
    /// retained, including output preferences; a new name is appended with default
    /// preferences. Values are stored literally.
    ///
    /// ```
    /// use mimecraft::ParameterList;
    ///
    /// let mut parameters = ParameterList::new();
    /// assert_eq!(parameters.set("FileName", "first.txt")?, None);
    /// assert_eq!(parameters.set("FILENAME", "second.txt")?, Some("first.txt".into()));
    /// assert_eq!(parameters.as_slice()[0].name(), "FileName");
    /// # Ok::<(), mimecraft::ParameterParseError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an invalid-name error with an offset in `name`. Failure leaves
    /// the list unchanged.
    pub fn set(
        &mut self,
        name: &str,
        value: impl Into<String>,
    ) -> Result<Option<String>, ParameterParseError> {
        validate_name(name)?;
        let value = value.into();
        if let Some(index) = self.index_of(name) {
            return Ok(Some(std::mem::replace(
                &mut self.parameters[index].value,
                value,
            )));
        }
        self.parameters.push(Parameter {
            name: name.to_owned(),
            value,
            encoding_method: None,
            charset: None,
            always_quote: false,
        });
        Ok(None)
    }

    /// Appends a parameter without replacing an existing name.
    ///
    /// # Errors
    ///
    /// Returns [`ParameterListEditError::DuplicateName`] for an existing name
    /// (ignoring ASCII case), leaving the list unchanged.
    pub fn add(&mut self, parameter: Parameter) -> Result<(), ParameterListEditError> {
        self.insert(self.len(), parameter)
    }

    /// Inserts a parameter before `index`, or appends when `index == len()`.
    ///
    /// # Errors
    ///
    /// Returns an error for an index greater than the length or a duplicate name,
    /// leaving the list unchanged.
    pub fn insert(
        &mut self,
        index: usize,
        parameter: Parameter,
    ) -> Result<(), ParameterListEditError> {
        if index > self.len() {
            return Err(ParameterListEditError::IndexOutOfBounds {
                index,
                len: self.len(),
            });
        }
        if self.contains(parameter.name()) {
            return Err(ParameterListEditError::DuplicateName(parameter.name));
        }
        self.parameters.insert(index, parameter);
        Ok(())
    }

    /// Replaces the parameter at `index`, returning the previous parameter.
    ///
    /// Unlike [`Self::set`], this adopts the replacement name's spelling.
    ///
    /// # Errors
    ///
    /// Returns an error for an out-of-range index or a name belonging to another
    /// entry, leaving the list unchanged.
    pub fn replace(
        &mut self,
        index: usize,
        parameter: Parameter,
    ) -> Result<Parameter, ParameterListEditError> {
        if index >= self.len() {
            return Err(ParameterListEditError::IndexOutOfBounds {
                index,
                len: self.len(),
            });
        }
        if self
            .index_of(parameter.name())
            .is_some_and(|existing| existing != index)
        {
            return Err(ParameterListEditError::DuplicateName(parameter.name));
        }
        Ok(std::mem::replace(&mut self.parameters[index], parameter))
    }

    /// Removes a named parameter, ignoring ASCII case, if present.
    ///
    /// The relative order of remaining entries and historical diagnostics are
    /// preserved.
    pub fn remove(&mut self, name: &str) -> Option<Parameter> {
        self.index_of(name)
            .map(|index| self.parameters.remove(index))
    }

    /// Removes and returns the parameter at `index`, preserving remaining order.
    ///
    /// # Errors
    ///
    /// Returns an error for an out-of-range index, leaving the list unchanged.
    pub fn remove_at(&mut self, index: usize) -> Result<Parameter, ParameterListEditError> {
        if index >= self.len() {
            return Err(ParameterListEditError::IndexOutOfBounds {
                index,
                len: self.len(),
            });
        }
        Ok(self.parameters.remove(index))
    }

    /// Removes all parameters, retaining historical parse diagnostics.
    pub fn clear(&mut self) {
        self.parameters.clear();
    }

    /// Returns whether a name is present, ignoring ASCII case.
    pub fn contains(&self, name: &str) -> bool {
        self.index_of(name).is_some()
    }

    /// Returns the position of a name, ignoring ASCII case.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.parameters
            .iter()
            .position(|parameter| parameter.name.eq_ignore_ascii_case(name))
    }

    /// Iterates over parameters in list order without allowing name replacement.
    pub fn iter(&self) -> std::slice::Iter<'_, Parameter> {
        self.parameters.iter()
    }

    // Typed header setters use fixed, known-valid parameter names.
    pub(crate) fn set_optional(&mut self, name: &str, value: Option<String>) {
        if let Some(value) = value {
            self.set(name, value).expect("known parameter name");
        } else {
            self.remove(name);
        }
    }

    /// Parses a complete parameter list using default compatibility options.
    ///
    /// # Errors
    ///
    /// Returns a syntax error with a byte offset relative to the supplied slice.
    /// Unterminated quotes and comments are errors in both modes, following
    /// MimeKit's throwing parse path. No partial list is returned on failure.
    pub fn parse(input: &[u8]) -> Result<Self, ParameterParseError> {
        Self::parse_with_options(input, &ParameterParseOptions::default())
    }

    /// Parses with explicit parameter, encoded-word, and raw-text options.
    ///
    /// Ordinary quoted values are unescaped and CR/LF removed. Incorrectly
    /// quoted extended values have only their outer quotes removed. Loose
    /// unquoted values retain internal spaces/tabs, discard CR/LF, and trim
    /// trailing whitespace.
    /// Empty entries between semicolons are ignored. `name=;` is accepted but
    /// `name=` at end of input is incomplete, matching MimeKit.
    ///
    /// Recovery diagnostics are available via [`Self::issues`]. Neither syntax
    /// mode validates percent/Base64/Q payloads or all RFC 2231 constraints.
    /// Parsed encoding preferences override header-wide formatting options.
    /// Ordinary values retain default preferences; quoting and language tags
    /// are not recovered.
    ///
    /// # Errors
    ///
    /// Returns [`ParameterParseError`] for invalid syntax, an unterminated quote
    /// or comment, or a continuation number greater than `i32::MAX`.
    pub fn parse_with_options(
        input: &[u8],
        options: &ParameterParseOptions,
    ) -> Result<Self, ParameterParseError> {
        Self::parse_from(input, 0, options)
    }

    // Keep positions relative to the whole header value when called by a typed
    // header parser, rather than rebasing errors and diagnostics after parsing.
    pub(crate) fn parse_from(
        input: &[u8],
        start: usize,
        options: &ParameterParseOptions,
    ) -> Result<Self, ParameterParseError> {
        let mut result = Self::default();
        let mut groups = parse_groups(input, start, options, &mut result.issues)?;
        result.parameters.reserve_exact(groups.len());
        // A single group cannot have duplicate definitions. Avoid allocating
        // a lookup table for the common one-parameter case.
        let mut positions = (groups.len() > 1)
            .then(|| HashMap::<ParameterName<'_>, usize>::with_capacity(groups.len()));
        for group in &mut groups {
            let parts = match group {
                PairGroup::Single(pair) => std::slice::from_mut(pair),
                PairGroup::Continuation(parts) => parts.as_mut_slice(),
            };
            let name = parts[0].name;
            let key = ParameterName(name);
            let existing = positions
                .as_ref()
                .and_then(|positions| positions.get(&key).copied());
            let continued = parts[0].id.is_some();
            if existing.is_some() {
                // Prefer the RFC 2231 form over an ordinary duplicate regardless
                // of encounter order. MIME gives no rule here; MimeKit follows
                // the RFC 6266 convention used by HTTP compatibility fallbacks.
                result.issues.push(ParameterIssue {
                    range: parts[0].range.clone(),
                    kind: ParameterIssueKind::DuplicateParameter,
                });
                if !continued && !parts[0].encoded {
                    continue;
                }
            }
            let (value, encoding_method, charset) = if continued || parts[0].encoded {
                if continued {
                    // Stable ties give duplicate section numbers a deterministic
                    // encounter order; the source's List.Sort tie order is unspecified.
                    parts.sort_by_key(|part| part.id);
                    check_sections(parts, &mut result.issues);
                }
                let (value, charset) = decode_extended(parts, options, &mut result.issues);
                (value, Some(ParameterEncodingMethod::Rfc2231), charset)
            } else {
                let (value, charset) = decode_ordinary(&parts[0], options, &mut result.issues);
                // UTF-8 is the default charset, so there is no need to remember
                // it. Still retain RFC 2047 to avoid switching encoding methods
                // on successive parse/format round trips.
                (
                    value,
                    charset.map(|_| ParameterEncodingMethod::Rfc2047),
                    charset.filter(|&charset| charset != Charset::UTF8),
                )
            };
            if let Some(index) = existing {
                let parameter = &mut result.parameters[index];
                parameter.value = value;
                parameter.encoding_method = encoding_method;
                parameter.charset = charset;
            } else {
                if let Some(positions) = &mut positions {
                    positions.insert(key, result.parameters.len());
                }
                result.parameters.push(Parameter {
                    name: name.to_owned(),
                    value,
                    encoding_method,
                    charset,
                    always_quote: false,
                });
            }
        }
        result.issues.sort_by_key(|issue| issue.range.start);
        Ok(result)
    }

    /// Returns decoded parameters in their current list order.
    pub fn as_slice(&self) -> &[Parameter] {
        &self.parameters
    }

    /// Looks up a decoded value using ASCII case-insensitive name comparison.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.index_of(name)
            .map(|index| self.parameters[index].value.as_str())
    }

    /// Returns the number of distinct parameter names.
    pub fn len(&self) -> usize {
        self.parameters.len()
    }

    /// Returns whether no parameters were supplied.
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty()
    }

    /// Returns recovery diagnostics in source order, including decoded values
    /// subsequently overridden by extended duplicates.
    ///
    /// Spans identify whole source values or bounding continuation regions,
    /// not individual invalid bytes. Out-of-order/interleaved continuations can
    /// make a span include unrelated parameters. Missing separators have an
    /// empty span at the next parameter's start.
    /// Edits, including [`Self::clear`], retain these original-input diagnostics.
    pub fn issues(&self) -> &[ParameterIssue] {
        &self.issues
    }
}

impl FromStr for ParameterList {
    type Err = ParameterParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.as_bytes())
    }
}

impl<'a> IntoIterator for &'a ParameterList {
    type Item = &'a Parameter;
    type IntoIter = std::slice::Iter<'a, Parameter>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An unsuccessful ordered parameter-list edit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParameterListEditError {
    /// Another parameter already has this name, ignoring ASCII case.
    DuplicateName(String),
    /// The supplied index is outside the range accepted by the operation.
    IndexOutOfBounds {
        /// The index supplied by the caller.
        index: usize,
        /// The current parameter count.
        len: usize,
    },
}

impl fmt::Display for ParameterListEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName(name) => write!(f, "parameter '{name}' already exists"),
            Self::IndexOutOfBounds { index, len } => {
                write!(
                    f,
                    "parameter index {index} is out of bounds for length {len}"
                )
            }
        }
    }
}

impl Error for ParameterListEditError {}

fn validate_name(name: &str) -> Result<(), ParameterParseError> {
    let invalid = if name.is_empty() {
        Some(0)
    } else {
        name.bytes().position(|byte| !is_attribute(byte))
    };
    if let Some(offset) = invalid {
        return Err(ParameterParseError::new(
            ParameterParseErrorKind::InvalidName,
            offset,
        ));
    }
    Ok(())
}

/// A recoverable parameter parsing or character-conversion event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterIssue {
    /// Half-open byte span relative to the original supplied input.
    pub range: Range<usize>,
    /// The compatibility or conversion behavior used.
    pub kind: ParameterIssueKind,
}

/// Recovery performed while decoding a parameter list.
///
/// These events are not an exhaustive RFC conformance report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParameterIssueKind {
    /// A missing semicolon was accepted in loose mode.
    MissingSeparator,
    /// An ordinary duplicate was ignored or an extended duplicate replaced it.
    DuplicateParameter,
    /// A repeated continuation section was retained and concatenated.
    DuplicateContinuation,
    /// The sorted sections did not start at zero or contained a gap.
    NonContiguousContinuation,
    /// An absent/incomplete RFC 2231 declaration defaulted to Latin-1.
    MissingCharset,
    /// An unsupported charset label required fallback.
    UnsupportedCharset {
        /// The charset selected instead of the declared label.
        selected: Charset,
    },
    /// Invalid bytes in a declared charset were replaced with U+FFFD.
    InvalidBytes {
        /// The declared charset used for decoding.
        charset: Charset,
    },
    /// Ordinary non-UTF-8 text required a fallback charset.
    UnlabelledText {
        /// The charset selected for the unlabelled bytes.
        selected: Charset,
    },
    /// A pending percent escape was discarded at a reset or end of the value.
    IncompletePercentEscape,
}

/// The reason a parameter list could not be parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterParseErrorKind {
    /// A nonempty ASCII attribute name was expected.
    InvalidName,
    /// A parameter suffix, equals sign, or value was incomplete.
    IncompleteParameter,
    /// A semicolon was required between parameters in strict mode.
    ExpectedSeparator,
    /// A continuation identifier exceeded `i32::MAX`.
    ContinuationOverflow,
    /// A quoted value did not have a closing double quote.
    IncompleteQuotedValue,
    /// A comment did not have a closing parenthesis.
    IncompleteComment,
}

/// A parameter syntax or name-validation failure with an input byte offset.
///
/// Parsing offsets refer to the supplied input slice. For [`Parameter::new`]
/// and [`ParameterList::set`], offsets refer to the parameter-name argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParameterParseError {
    kind: ParameterParseErrorKind,
    offset: usize,
}

impl ParameterParseError {
    pub(crate) fn new(kind: ParameterParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// Returns the kind of syntax error.
    pub fn kind(self) -> ParameterParseErrorKind {
        self.kind
    }

    /// Returns the failure's byte offset, possibly equal to the input length.
    ///
    /// Unterminated comments/quotes point to the opening delimiter. Overflow
    /// points to the first digit that would exceed the limit.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for ParameterParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            ParameterParseErrorKind::InvalidName => "expected a parameter name",
            ParameterParseErrorKind::IncompleteParameter => "incomplete parameter",
            ParameterParseErrorKind::ExpectedSeparator => "expected a parameter separator",
            ParameterParseErrorKind::ContinuationOverflow => "continuation number exceeds i32::MAX",
            ParameterParseErrorKind::IncompleteQuotedValue => "unterminated quoted parameter value",
            ParameterParseErrorKind::IncompleteComment => "unterminated comment",
        };
        write!(f, "{message} at byte {}", self.offset)
    }
}

impl Error for ParameterParseError {}

struct Pair<'a> {
    name: &'a str,
    id: Option<u32>,
    encoded: bool,
    value: Cow<'a, [u8]>,
    value_range: Range<usize>,
    range: Range<usize>,
}

// Names are ASCII. Fold borrowed names for hashing and comparison instead of
// allocating lowercase keys independently of the final parameter names.
struct ParameterName<'a>(&'a str);

impl PartialEq for ParameterName<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(other.0)
    }
}

impl Eq for ParameterName<'_> {}

impl Hash for ParameterName<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for byte in self.0.bytes() {
            state.write_u8(byte.to_ascii_lowercase());
        }
    }
}

enum PairGroup<'a> {
    Single(Pair<'a>),
    Continuation(Vec<Pair<'a>>),
}

fn skip_cfws(input: &[u8], index: &mut usize) -> Result<(), ParameterParseError> {
    skip_comments_and_whitespace(input, index).map_err(|offset| {
        ParameterParseError::new(ParameterParseErrorKind::IncompleteComment, offset)
    })
}

fn require_content(input: &[u8], index: &mut usize) -> Result<(), ParameterParseError> {
    skip_cfws(input, index)?;
    if *index == input.len() {
        return Err(ParameterParseError::new(
            ParameterParseErrorKind::IncompleteParameter,
            *index,
        ));
    }
    Ok(())
}

fn parse_pair<'a>(
    input: &'a [u8],
    index: &mut usize,
    mode: ParameterComplianceMode,
) -> Result<Pair<'a>, ParameterParseError> {
    let start = *index;
    while input.get(*index).is_some_and(|&byte| is_attribute(byte)) {
        *index += 1;
    }
    if *index == start {
        return Err(ParameterParseError::new(
            ParameterParseErrorKind::InvalidName,
            start,
        ));
    }
    let name = std::str::from_utf8(&input[start..*index]).expect("attribute names are ASCII");
    require_content(input, index)?;
    let mut id = None;
    let mut encoded = false;
    if input[*index] == b'*' {
        *index += 1;
        require_content(input, index)?;
        if input[*index].is_ascii_digit() {
            let mut number = 0u32;
            while let Some(&byte) = input.get(*index).filter(|byte| byte.is_ascii_digit()) {
                let digit = u32::from(byte - b'0');
                if number > (i32::MAX as u32 - digit) / 10 {
                    return Err(ParameterParseError::new(
                        ParameterParseErrorKind::ContinuationOverflow,
                        *index,
                    ));
                }
                number = number * 10 + digit;
                *index += 1;
            }
            id = Some(number);
            require_content(input, index)?;
            if input[*index] == b'*' {
                encoded = true;
                *index += 1;
                require_content(input, index)?;
            }
        } else {
            encoded = true;
        }
    }
    if input[*index] != b'=' {
        return Err(ParameterParseError::new(
            ParameterParseErrorKind::IncompleteParameter,
            *index,
        ));
    }
    *index += 1;
    require_content(input, index)?;
    let value_start = *index;
    let value = if input[*index] == b'"' {
        skip_quoted(input, index).map_err(|offset| {
            ParameterParseError::new(ParameterParseErrorKind::IncompleteQuotedValue, offset)
        })?;
        Cow::Borrowed(&input[value_start..*index])
    } else if mode == ParameterComplianceMode::Strict {
        while input.get(*index).is_some_and(|&byte| is_token(byte)) {
            *index += 1;
        }
        Cow::Borrowed(&input[value_start..*index])
    } else {
        // Some senders leave filenames with spaces and folded lines unquoted.
        // See MimeKit issues #106 (Google Docs) and #159 (folded values).
        while *index < input.len() && input[*index] != b';' {
            *index += 1;
        }
        let bytes = &input[value_start..*index];
        // Both paths trim trailing whitespace without removing internal spaces,
        // preserving the compatibility workaround from MimeKit issue #278.
        let end = bytes
            .iter()
            .rposition(|&byte| !is_whitespace(byte))
            .map_or(0, |i| i + 1);
        let bytes = &bytes[..end];
        if bytes.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            let mut unfolded = Vec::with_capacity(bytes.len());
            unfolded.extend(
                bytes
                    .iter()
                    .copied()
                    .filter(|byte| !matches!(byte, b'\r' | b'\n')),
            );
            Cow::Owned(unfolded)
        } else {
            Cow::Borrowed(bytes)
        }
    };
    Ok(Pair {
        name,
        id,
        encoded,
        value,
        value_range: value_start..*index,
        range: start..*index,
    })
}

fn parse_groups<'a>(
    input: &'a [u8],
    start: usize,
    options: &ParameterParseOptions,
    issues: &mut Vec<ParameterIssue>,
) -> Result<Vec<PairGroup<'a>>, ParameterParseError> {
    let mut groups = Vec::new();
    let mut continuations = HashMap::<ParameterName<'_>, usize>::new();
    let mut index = start;
    loop {
        skip_cfws(input, &mut index)?;
        if index == input.len() {
            break;
        }
        if input[index] == b';' {
            index += 1;
            continue;
        }
        let pair = parse_pair(input, &mut index, options.compliance_mode)?;
        skip_cfws(input, &mut index)?;
        // A complete single pair needs neither spare group capacity nor a
        // continuation index, even when it has a section number.
        if groups.is_empty() && index == input.len() {
            return Ok(vec![PairGroup::Single(pair)]);
        }
        if pair.id.is_some() {
            let key = ParameterName(pair.name);
            if let Some(&group) = continuations.get(&key) {
                let PairGroup::Continuation(parts) = &mut groups[group] else {
                    unreachable!("only continuation groups are indexed");
                };
                parts.push(pair);
            } else {
                continuations.insert(key, groups.len());
                groups.push(PairGroup::Continuation(vec![pair]));
            }
        } else {
            // Like MimeKit's flat parameter list, ordinary definitions do not
            // need a separately allocated list of continuation sections.
            groups.push(PairGroup::Single(pair));
        }
        if index < input.len() && input[index] != b';' {
            if options.compliance_mode == ParameterComplianceMode::Strict {
                return Err(ParameterParseError::new(
                    ParameterParseErrorKind::ExpectedSeparator,
                    index,
                ));
            }
            issues.push(ParameterIssue {
                range: index..index,
                kind: ParameterIssueKind::MissingSeparator,
            });
        }
    }
    Ok(groups)
}

fn check_sections(parts: &[Pair<'_>], issues: &mut Vec<ParameterIssue>) {
    let mut previous = None;
    for part in parts {
        let id = part.id.expect("continuation group");
        let kind = if previous == Some(id) {
            Some(ParameterIssueKind::DuplicateContinuation)
        } else if id != previous.map_or(0, |id| id + 1) {
            Some(ParameterIssueKind::NonContiguousContinuation)
        } else {
            None
        };
        if let Some(kind) = kind {
            issues.push(ParameterIssue {
                range: part.range.clone(),
                kind,
            });
        }
        previous = Some(id);
    }
}

fn decode_ordinary(
    part: &Pair<'_>,
    options: &ParameterParseOptions,
    issues: &mut Vec<ParameterIssue>,
) -> (String, Option<Charset>) {
    let bytes = if part.value.starts_with(b"\"") {
        unquote_bytes(&part.value, false)
    } else {
        Cow::Borrowed(part.value.as_ref())
    };
    let (decoded, charset) = rfc2047::decode_parameter(
        &bytes,
        &rfc2047::DecodeOptions {
            compliance_mode: options.rfc2047_compliance_mode,
            fallback_charset: options.fallback_charset,
        },
    );
    for issue in decoded.issues {
        let kind = match issue {
            DecodeIssue::UnsupportedCharset { selected, .. } => {
                ParameterIssueKind::UnsupportedCharset { selected }
            }
            DecodeIssue::InvalidBytes { charset, .. } => {
                ParameterIssueKind::InvalidBytes { charset }
            }
            DecodeIssue::UnlabelledText { selected, .. } => {
                ParameterIssueKind::UnlabelledText { selected }
            }
        };
        // Unquoting/unfolding changes byte positions. Report the original whole
        // value instead of pretending transformed encoded-word offsets are raw.
        issues.push(ParameterIssue {
            range: part.value_range.clone(),
            kind,
        });
    }
    (decoded.text, charset)
}

fn extended_charset<'a>(
    bytes: &'a [u8],
    range: Range<usize>,
    issues: &mut Vec<ParameterIssue>,
) -> (Charset, &'a [u8]) {
    if let Some(first) = bytes
        .iter()
        .position(|&byte| byte == b'\'')
        .filter(|&i| i > 0)
        && let Some(second) = bytes[first + 1..].iter().position(|&byte| byte == b'\'')
    {
        let payload = &bytes[first + second + 2..];
        let charset = std::str::from_utf8(&bytes[..first])
            .ok()
            .and_then(|label| Charset::from_label(label).ok());
        if let Some(charset) = charset {
            return (charset, payload);
        }
        issues.push(ParameterIssue {
            range,
            kind: ParameterIssueKind::UnsupportedCharset {
                selected: Charset::LATIN1,
            },
        });
        return (Charset::LATIN1, payload);
    }
    // A missing declaration ought to imply ASCII, but broken mail contains
    // high bytes here. Latin-1 preserves them instead of replacing them.
    issues.push(ParameterIssue {
        range,
        kind: ParameterIssueKind::MissingCharset,
    });
    (Charset::LATIN1, bytes)
}

fn decode_extended(
    parts: &[Pair<'_>],
    options: &ParameterParseOptions,
    issues: &mut Vec<ParameterIssue>,
) -> (String, Option<Charset>) {
    // MimeKit estimates the assembled value's capacity from its raw sections.
    // Reserve the encoded bytes separately; ordinary sections need no scratch.
    let capacity = parts.iter().map(|part| part.value.len()).sum();
    let encoded_capacity = parts
        .iter()
        .filter(|part| part.encoded)
        .map(|part| part.value.len())
        .sum();
    let mut output = String::with_capacity(capacity);
    let mut hex = Rfc2231Decoder::new();
    let mut charset: Option<Charset> = None;
    let mut bytes = Vec::with_capacity(encoded_capacity);
    let mut run_range: Option<Range<usize>> = None;
    let group_range = parts
        .iter()
        .map(|part| part.range.start)
        .min()
        .expect("nonempty group")
        ..parts
            .iter()
            .map(|part| part.range.end)
            .max()
            .expect("nonempty group");
    for (index, part) in parts.iter().enumerate() {
        if part.encoded {
            // Broken senders quote extended values. Strip only outer quotes,
            // not quoted pairs, matching the source's compatibility workaround.
            let mut payload = part.value.as_ref();
            if payload.len() >= 2 && payload.starts_with(b"\"") && payload.ends_with(b"\"") {
                payload = &payload[1..payload.len() - 1];
            }
            if charset.is_none() {
                // Only the first encoded section declares the charset; later
                // sections reuse it even when unencoded sections intervene.
                let (selected, remaining) =
                    extended_charset(payload, part.value_range.clone(), issues);
                charset = Some(selected);
                payload = remaining;
            }
            if charset.is_some_and(|charset| charset.ends_in_iso2022_ascii_reset(&bytes)) {
                flush_extended_bytes(
                    &mut output,
                    &mut bytes,
                    charset.expect("selected charset"),
                    &mut run_range,
                    issues,
                );
            }
            // Carry percent-escape state and decoded bytes across adjacent
            // encoded sections: a boundary can fall inside a multibyte character.
            hex.decode(payload, &mut bytes);
            if let Some(range) = &mut run_range {
                range.start = range.start.min(part.value_range.start);
                range.end = range.end.max(part.value_range.end);
            } else {
                run_range = Some(part.value_range.clone());
            }
            if !parts.get(index + 1).is_some_and(|next| next.encoded) {
                flush_extended_bytes(
                    &mut output,
                    &mut bytes,
                    charset.expect("encoded segment selected a charset"),
                    &mut run_range,
                    issues,
                );
            }
        } else if !part.value.is_empty() {
            let (text, selected) = decode_best(&part.value, options.fallback_charset);
            if selected != Charset::UTF8 {
                issues.push(ParameterIssue {
                    range: part.value_range.clone(),
                    kind: ParameterIssueKind::UnlabelledText { selected },
                });
            }
            if part.value.starts_with(b"\"") {
                output.push_str(&unquote(&text, false));
            } else {
                output.push_str(&text);
            }
            if hex.has_pending_escape() {
                issues.push(ParameterIssue {
                    range: group_range.clone(),
                    kind: ParameterIssueKind::IncompletePercentEscape,
                });
            }
            hex.reset();
        }
        // An empty unquoted section deliberately does not reset percent state.
        // Character state, however, was flushed at the preceding encoded run.
    }
    if hex.has_pending_escape() {
        issues.push(ParameterIssue {
            range: group_range,
            kind: ParameterIssueKind::IncompletePercentEscape,
        });
    }
    (output, charset)
}

fn flush_extended_bytes(
    output: &mut String,
    bytes: &mut Vec<u8>,
    charset: Charset,
    run_range: &mut Option<Range<usize>>,
    issues: &mut Vec<ParameterIssue>,
) {
    let range = run_range.take().expect("encoded run");
    let (text, had_errors) = charset.decode_lossy(bytes);
    output.push_str(&text);
    if had_errors {
        issues.push(ParameterIssue {
            range,
            kind: ParameterIssueKind::InvalidBytes { charset },
        });
    }
    bytes.clear();
}

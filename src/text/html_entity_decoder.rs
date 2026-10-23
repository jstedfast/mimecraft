// Adapted from MimeKit's Text/HtmlEntityDecoder.cs and Text/HtmlUtils.cs.
// Reporting also adapts Text/HtmlTokenizer.cs, HtmlComplianceOptions.cs,
// HtmlComplianceViolation.cs, and HtmlCappedComplianceLogger.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, ops::Range};

use super::html_entities::{MAX_NAME_LENGTH, NAMED_ENTITIES};

const MAX_CODE_POINT: u32 = 0x10ffff;
const MAX_ENTITY_LENGTH: usize = MAX_NAME_LENGTH + 1;

/// A character-reference compliance violation, not a general HTML parse error.
///
/// Names follow MimeKit's character-reference diagnostics. Numeric values are
/// not a compatibility contract. The truncation marker describes an incomplete
/// report rather than a defect in the input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HtmlEntityViolation {
    /// A numeric prefix (`&#` or `&#x`) has no digits and remains literal.
    AbsenceOfDigitsInNumericCharacterReference,
    /// A numeric value exceeds U+10FFFF and decodes as U+FFFD.
    CharacterReferenceOutsideUnicodeRange,
    /// A numeric value denotes a control character, before any C1 remapping.
    ///
    /// TAB, LF, and FF are exempt, but CR is not. Zero has its own violation.
    ControlCharacterReference,
    /// A decoded numeric or legacy named reference lacks its final `;`.
    ///
    /// Ambiguous legacy names preserved by attribute-context rules are exempt.
    MissingSemicolonAfterCharacterReference,
    /// A numeric value denotes a Unicode noncharacter, emitted unchanged.
    NoncharacterCharacterReference,
    /// A numeric value denotes U+0000 and decodes as U+FFFD.
    NullCharacterReference,
    /// A numeric value denotes a surrogate and decodes as U+FFFD.
    SurrogateCharacterReference,
    /// An ASCII alphanumeric name ends with `;` but has no known matching prefix.
    UnknownNamedCharacterReference,
    /// The first issue suppressed by a per-violation reporting limit.
    ///
    /// Emitted once per whole-string call, at the suppressed issue's offset.
    TooManyComplianceIssues,
}

/// A reference diagnostic located at its leading `&` in the original input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HtmlDecodeIssue {
    /// The reference violation, or report-truncation marker.
    pub violation: HtmlEntityViolation,
    /// Zero-based UTF-8 byte offset, not MimeKit's UTF-16 character offset.
    ///
    /// Offsets refer to the supplied string, not the decoded output. No BOM,
    /// whitespace, line-ending, or quote removal takes place.
    pub offset: usize,
}

/// Options for opt-in whole-string character-reference diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HtmlDecodeOptions {
    /// Maximum reports for each violation; zero (the default) means unlimited.
    ///
    /// The budget is per call and per violation, so one kind cannot hide other
    /// kinds. The first suppressed issue produces one truncation marker.
    /// For a positive limit, at most eight times this limit plus one issues
    /// are stored. Decoding continues unchanged after a budget is exhausted.
    pub max_issues_per_violation: usize,
}

/// Decoded text and opt-in reference diagnostics in source order.
///
/// At a shared offset, a missing semicolon precedes a numeric-value violation.
/// An empty issue list does not certify valid or safe HTML: only character
/// references are checked, not tags, attributes, literal controls, or URLs.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct DecodedHtml {
    /// The same decoded text produced by the corresponding non-reporting API.
    pub text: String,
    /// Reference diagnostics, subject to the requested per-violation budget.
    pub issues: Vec<HtmlDecodeIssue>,
}

/// An invalid first character supplied to an [`HtmlEntityDecoder`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlEntityDecoderError {
    /// The first character after construction or reset was not `&`.
    ExpectedAmpersand,
}

impl fmt::Display for HtmlEntityDecoderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an HTML character reference must begin with '&'")
    }
}

impl Error for HtmlEntityDecoderError {}

/// Incrementally decodes one HTML character reference at a time.
///
/// Push the leading `&` and subsequent characters until [`Self::push`] returns
/// `false`, or the input ends. The rejected character is not consumed: process
/// it separately and, in attribute context, pass it to [`Self::attribute_value`].
/// Read the value and [`reset`](Self::reset) before starting another reference.
/// An accepted semicolon completes the reference; further characters are refused.
///
/// State survives input chunk boundaries. Do not reset or finalize just because
/// a chunk ended: a later semicolon can turn a partial legacy match into a
/// longer name. This decodes references only, not surrounding text or markup.
///
/// Named lookup uses the generated, case-sensitive entity table. Numeric
/// recovery matches [`html_decode`], consuming arbitrarily long digit runs
/// with bounded storage. [`Self::violations`] and [`Self::attribute_violations`]
/// inspect the current reference without changing decoding. No stream I/O is
/// provided.
///
/// ```
/// use mimecraft::text::HtmlEntityDecoder;
///
/// let mut decoder = HtmlEntityDecoder::new();
/// for character in "&noti".chars() {
///     assert!(decoder.push(character)?);
/// }
/// assert_eq!(decoder.value(), "\u{ac}i");
/// assert_eq!(decoder.attribute_value(None), "&noti");
/// for character in "n;".chars() {
///     assert!(decoder.push(character)?);
/// }
/// assert_eq!(decoder.value(), "\u{2209}");
/// assert!(!decoder.push('!')?); // The caller still owns this character.
/// decoder.reset();
/// assert_eq!(decoder.value(), "");
/// # Ok::<(), mimecraft::text::HtmlEntityDecoderError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlEntityDecoder {
    pushed: String,
    named_range: Range<usize>,
    matched: Option<usize>,
    numeric_value: u32,
    radix: u32,
    numeric: bool,
    digits: bool,
    semicolon: bool,
}

impl Default for HtmlEntityDecoder {
    fn default() -> Self {
        Self {
            pushed: String::with_capacity(MAX_ENTITY_LENGTH),
            named_range: 0..NAMED_ENTITIES.len(),
            matched: None,
            numeric_value: 0,
            radix: 0,
            numeric: false,
            digits: false,
            semicolon: false,
        }
    }
}

impl HtmlEntityDecoder {
    /// Creates a decoder ready for the leading `&` of a reference.
    pub fn new() -> Self {
        Self::default()
    }

    /// Tries to consume one character, returning `false` when it is not part
    /// of the current reference or a semicolon has already ended it.
    ///
    /// The first character must be `&`. A refused character is not included
    /// in the value; read the value and reset before processing a new reference.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlEntityDecoderError::ExpectedAmpersand`] for an invalid
    /// first character, leaving the initial state unchanged.
    pub fn push(&mut self, character: char) -> Result<bool, HtmlEntityDecoderError> {
        if self.semicolon {
            return Ok(false);
        }
        if self.pushed.is_empty() {
            if character != '&' {
                return Err(HtmlEntityDecoderError::ExpectedAmpersand);
            }
            self.pushed.push('&');
            return Ok(true);
        }
        if self.numeric {
            return Ok(self.push_numeric(character));
        }
        if self.pushed.len() == 1 && character == '#' {
            self.pushed.push('#');
            self.numeric = true;
            return Ok(true);
        }
        if !character.is_ascii() || self.pushed.len() == MAX_ENTITY_LENGTH {
            return Ok(false);
        }

        // Each accepted name prefix narrows a contiguous range in the sorted
        // table. Entries ending at that prefix sort before their extensions.
        // This replaces the generated transition tables without allocating
        // lookup keys or rescanning names that no longer match.
        let position = self.pushed.len() - 1;
        let byte = character as u8;
        let candidates = &NAMED_ENTITIES[self.named_range.clone()];
        let lower = candidates.partition_point(|(name, _)| {
            name.as_bytes()
                .get(position)
                .is_none_or(|&next| next < byte)
        });
        let upper = candidates.partition_point(|(name, _)| {
            name.as_bytes()
                .get(position)
                .is_none_or(|&next| next <= byte)
        });
        if lower == upper {
            return Ok(false);
        }
        let start = self.named_range.start;
        self.named_range = start + lower..start + upper;
        self.pushed.push(character);
        if NAMED_ENTITIES[self.named_range.start].0.len() == position + 1 {
            self.matched = Some(self.named_range.start);
        }
        self.semicolon = character == ';';
        Ok(true)
    }

    fn push_numeric(&mut self, character: char) -> bool {
        if self.radix == 0 {
            if matches!(character, 'x' | 'X') {
                self.pushed.push(character);
                self.radix = 16;
                return true;
            }
            self.radix = 10;
        }
        if character == ';' {
            if !self.digits {
                return false;
            }
            self.semicolon = true;
            self.append_numeric(character);
            return true;
        }
        let Some(digit) = character.to_digit(self.radix) else {
            return false;
        };
        // Consume every digit, regardless of length. Saturate just above the
        // maximum code point to avoid overflow while retaining invalid-range
        // information for numeric character reference recovery.
        self.numeric_value = (self.numeric_value * self.radix + digit).min(MAX_CODE_POINT + 1);
        self.append_numeric(character);
        self.digits = true;
        true
    }

    fn append_numeric(&mut self, character: char) {
        // Retain only the leading characters of an arbitrarily long digit
        // run. Raw text is only needed if no digits were consumed (e.g. "&#x"),
        // in which case it is always short.
        if self.pushed.len() < MAX_ENTITY_LENGTH {
            self.pushed.push(character);
        }
    }

    /// Returns the current character-data value without consuming or resetting.
    ///
    /// Any pushed suffix beyond the longest named match remains literal.
    /// Unmatched names and digitless numeric prefixes retain their raw spelling.
    /// An unused decoder returns an empty string. The returned string is owned.
    pub fn value(&self) -> String {
        if self.numeric && self.digits {
            return numeric_value(self.numeric_value).to_string();
        }
        if let Some(index) = self.matched {
            let (name, value) = NAMED_ENTITIES[index];
            let mut output = value.to_owned();
            output.push_str(&self.pushed[name.len() + 1..]);
            output
        } else {
            self.pushed.clone()
        }
    }

    /// Returns the value under HTML attribute-context rules.
    ///
    /// Supply the first unconsumed character, or `None` at the end of input.
    /// A suffix already pushed beyond the longest match takes precedence over
    /// that lookahead. Legacy names without `;` stay literal when followed by
    /// ASCII alphanumeric characters or `=`; numeric references are unaffected.
    pub fn attribute_value(&self, next: Option<char>) -> String {
        if self.historical_attribute_value(next) {
            self.pushed.clone()
        } else {
            self.value()
        }
    }

    fn historical_attribute_value(&self, next: Option<char>) -> bool {
        self.matched.is_some_and(|index| {
            let name = NAMED_ENTITIES[index].0;
            let next = self.pushed[name.len() + 1..].chars().next().or(next);
            is_historical_attribute_value(name, next)
        })
    }

    /// Inspects the current reference under character-data rules.
    ///
    /// Read after a refused character or final end-of-input, not a temporary
    /// chunk boundary. Reports at most two violations, without allocating or
    /// changing state; a missing semicolon precedes a numeric-value violation.
    /// An unused decoder has no violations.
    ///
    /// Unknown names require scanning characters that this decoder refuses,
    /// so they are not reported here. [`html_decode_with_options`] performs
    /// that additional scan and attaches original-input byte offsets.
    pub fn violations(&self) -> impl Iterator<Item = HtmlEntityViolation> {
        self.compliance_violations(false, None)
            .into_iter()
            .flatten()
    }

    /// Inspects the current reference under attribute-context rules.
    ///
    /// `next` is the first unconsumed character, or `None` at final end-of-input.
    /// A pushed suffix takes precedence over that lookahead, just as in
    /// [`Self::attribute_value`]. Ambiguous legacy references preserved literally
    /// do not produce missing-semicolon violations. Other behavior and the
    /// unknown-name limitation are the same as [`Self::violations`].
    pub fn attribute_violations(
        &self,
        next: Option<char>,
    ) -> impl Iterator<Item = HtmlEntityViolation> {
        self.compliance_violations(true, next).into_iter().flatten()
    }

    fn compliance_violations(
        &self,
        in_attribute: bool,
        next: Option<char>,
    ) -> [Option<HtmlEntityViolation>; 2] {
        use HtmlEntityViolation::*;

        if self.numeric {
            if !self.digits {
                return [Some(AbsenceOfDigitsInNumericCharacterReference), None];
            }
            let violation = numeric_violation(self.numeric_value);
            if self.semicolon {
                [violation, None]
            } else {
                [Some(MissingSemicolonAfterCharacterReference), violation]
            }
        } else if self.matched.is_some()
            && !self.semicolon
            && !(in_attribute && self.historical_attribute_value(next))
        {
            [Some(MissingSemicolonAfterCharacterReference), None]
        } else {
            [None, None]
        }
    }

    fn unknown_named_length(&self) -> Option<usize> {
        if self.numeric || self.matched.is_some() || self.pushed.is_empty() {
            None
        } else {
            Some(self.pushed.len() - 1)
        }
    }

    /// Resets all reference state while retaining the allocated input buffer.
    pub fn reset(&mut self) {
        self.pushed.clear();
        self.named_range = 0..NAMED_ENTITIES.len();
        self.matched = None;
        self.numeric_value = 0;
        self.radix = 0;
        self.numeric = false;
        self.digits = false;
        self.semicolon = false;
    }
}

/// Decodes HTML character references in a complete Unicode string.
///
/// Uses MimeKit's full, case-sensitive named-entity table, choosing the longest
/// matching name. Only explicitly listed legacy names accept a missing
/// semicolon. Unknown names and unmatched suffixes remain literal.
/// Decimal (`&#65;`) and hexadecimal (`&#x41;` or `&#X41;`) references accept
/// missing semicolons and unbounded digit runs, including leading zeros.
///
/// Numeric zero, surrogate values, and values beyond U+10FFFF become U+FFFD.
/// The HTML Windows-1252 compatibility mappings apply to selected C1 controls;
/// other control characters and Unicode noncharacters are retained. Numeric
/// references without digits remain literal. These are intentional recovery
/// rules, not strict validation; no compliance diagnostics are returned.
/// Use [`html_decode_with_options`] to collect reference diagnostics.
///
/// This uses character-data rules, not attribute-context rules: `&notit;`
/// becomes `\u{ac}it;`. Quotes are not removed. Decoding is a single pass:
/// `&amp;lt;` becomes `&lt;`, not `<`. Literal text, markup, whitespace, and
/// newlines are otherwise unchanged. Does not render, sanitize, or parse HTML,
/// decode a charset, or perform stream I/O.
/// Use [`html_attribute_decode`] for the contents of an HTML attribute.
///
/// ```
/// use mimecraft::text::html_decode;
///
/// assert_eq!(html_decode("&lt;b&gt;caf&#233;&lt;/b&gt;"), "<b>caf\u{e9}</b>");
/// assert_eq!(html_decode("&notin; &notit;"), "\u{2209} \u{ac}it;");
/// assert_eq!(html_decode("&#x110000; &unknown;"), "\u{fffd} &unknown;");
/// ```
pub fn html_decode(text: &str) -> String {
    decode(text, false)
}

/// Decodes references in an HTML attribute's contents, without removing quotes.
///
/// Like [`html_decode`], except a legacy named reference without `;` remains
/// literal if followed by `=` or an ASCII letter/digit. Semicolon-terminated
/// names and numeric references use the ordinary decoding rules.
///
/// This does not parse an attribute or tag, normalize whitespace, replace
/// literal NUL characters, or distinguish quoted from unquoted syntax.
/// Pass the attribute contents rather than a complete HTML document.
/// Decoding is one pass and is not sanitization or URL validation.
/// Use [`html_attribute_decode_with_options`] to collect reference diagnostics.
///
/// ```
/// use mimecraft::text::html_attribute_decode;
///
/// assert_eq!(html_attribute_decode("?a=1&not=2"), "?a=1&not=2");
/// assert_eq!(html_attribute_decode("&notit; &not! &notin;"), "&notit; \u{ac}! \u{2209}");
/// assert_eq!(html_attribute_decode("&#65= &amp;lt;"), "A= &lt;");
/// ```
pub fn html_attribute_decode(text: &str) -> String {
    decode(text, true)
}

/// Decodes character data and reports reference compliance violations.
///
/// Output matches [`html_decode`]; reporting never changes recovery. Each issue
/// points to its reference's leading `&` using a UTF-8 byte offset. A missing
/// semicolon is reported before any numeric-value violation at that offset.
/// Unknown names are reported only when an ASCII alphanumeric name reaches `;`
/// without a known matching prefix.
///
/// Unlike an HTML tokenizer, this treats the entire input as character data,
/// even inside text resembling tags, comments, or script. It does not validate
/// literal controls or provide general HTML compliance checks.
///
/// The default limit is unlimited; set a positive per-violation limit to bound
/// report size for untrusted input. One truncation marker identifies the first
/// suppressed issue. Limits never carry over to another call.
///
/// ```
/// use mimecraft::text::{HtmlDecodeOptions, HtmlEntityViolation, html_decode_with_options};
///
/// let decoded = html_decode_with_options("x &#x110000", &HtmlDecodeOptions::default());
/// assert_eq!(decoded.text, "x \u{fffd}");
/// assert_eq!(decoded.issues[0].offset, 2);
/// assert_eq!(decoded.issues[0].violation,
///     HtmlEntityViolation::MissingSemicolonAfterCharacterReference);
/// assert_eq!(decoded.issues[1].violation,
///     HtmlEntityViolation::CharacterReferenceOutsideUnicodeRange);
/// ```
pub fn html_decode_with_options(text: &str, options: &HtmlDecodeOptions) -> DecodedHtml {
    decode_with_options(text, options, false)
}

/// Decodes attribute contents and reports reference compliance violations.
///
/// Output matches [`html_attribute_decode`]. Like [`html_decode_with_options`],
/// but ambiguous semicolonless legacy names remain literal without a violation.
/// Supply attribute contents, not a complete tag. Quotes are not removed and
/// byte offsets are relative to the supplied string.
///
/// ```
/// use mimecraft::text::{HtmlDecodeOptions, HtmlEntityViolation, html_attribute_decode_with_options};
///
/// let options = HtmlDecodeOptions { max_issues_per_violation: 1 };
/// let decoded = html_attribute_decode_with_options("&copy=1 &#0;&#0;", &options);
/// assert_eq!(decoded.text, "&copy=1 \u{fffd}\u{fffd}");
/// assert_eq!(decoded.issues[0].violation, HtmlEntityViolation::NullCharacterReference);
/// assert_eq!(decoded.issues[1].violation, HtmlEntityViolation::TooManyComplianceIssues);
/// assert_eq!(decoded.issues[1].offset, 12);
/// ```
pub fn html_attribute_decode_with_options(text: &str, options: &HtmlDecodeOptions) -> DecodedHtml {
    decode_with_options(text, options, true)
}

fn decode_with_options(text: &str, options: &HtmlDecodeOptions, in_attribute: bool) -> DecodedHtml {
    let mut output = String::with_capacity(text.len());
    let mut issues = Vec::new();
    // One counter per violation in a dense, zero-based range. The truncation
    // marker is not a document defect and is not counted against any budget.
    let mut counts = [0usize; HtmlEntityViolation::TooManyComplianceIssues as usize];
    let mut reported_truncation = false;
    let mut report = |violation: HtmlEntityViolation, offset| {
        if options.max_issues_per_violation != 0 {
            let count = &mut counts[violation as usize];
            if *count == options.max_issues_per_violation {
                // The report is incomplete from the first suppressed issue
                // onward. Emit the marker there only once, not once per kind:
                // repeated markers would reintroduce the flood being limited.
                if !reported_truncation {
                    reported_truncation = true;
                    issues.push(HtmlDecodeIssue {
                        violation: HtmlEntityViolation::TooManyComplianceIssues,
                        offset,
                    });
                }
                return;
            }
            *count += 1;
        }
        issues.push(HtmlDecodeIssue { violation, offset });
    };

    let mut decoder = HtmlEntityDecoder::new();
    let mut remaining = text;
    while let Some(index) = remaining.find('&') {
        let offset = text.len() - remaining.len() + index;
        output.push_str(&remaining[..index]);
        remaining = &remaining[index + 1..];
        decoder.reset();
        decoder.push('&').expect("a reference starts with '&'");
        let mut consumed = 0;
        for character in remaining.chars() {
            if !decoder.push(character).expect("the reference has begun") {
                break;
            }
            consumed += character.len_utf8();
            if character == ';' {
                break;
            }
        }
        remaining = &remaining[consumed..];
        let next = remaining.chars().next();
        output.push_str(&if in_attribute {
            decoder.attribute_value(next)
        } else {
            decoder.value()
        });
        for violation in decoder
            .compliance_violations(in_attribute, next)
            .into_iter()
            .flatten()
        {
            report(violation, offset);
        }

        // Emulate the ambiguous ampersand state solely to diagnose unknown
        // names. These ASCII alphanumerics would otherwise be copied as
        // ordinary text anyway; a terminating ';' is required for an issue.
        if let Some(length) = decoder.unknown_named_length() {
            let suffix = remaining
                .bytes()
                .take_while(u8::is_ascii_alphanumeric)
                .count();
            if length + suffix > 0 && remaining.as_bytes().get(suffix) == Some(&b';') {
                report(HtmlEntityViolation::UnknownNamedCharacterReference, offset);
            }
        }
    }
    output.push_str(remaining);
    DecodedHtml {
        text: output,
        issues,
    }
}

fn decode(text: &str, in_attribute: bool) -> String {
    let mut output = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(index) = remaining.find('&') {
        output.push_str(&remaining[..index]);
        remaining = &remaining[index + 1..];
        if let Some((length, character)) = numeric_reference(remaining) {
            output.push(character);
            remaining = &remaining[length..];
        } else if let Some((length, value)) = named_reference(remaining, in_attribute) {
            output.push_str(value);
            remaining = &remaining[length..];
        } else {
            output.push('&');
        }
    }
    output.push_str(remaining);
    output
}

fn named_reference(text: &str, in_attribute: bool) -> Option<(usize, &'static str)> {
    let mut length = 0;
    // Names are bounded ASCII strings. Do not rescan an arbitrarily long
    // unknown name or slice through a UTF-8 character.
    for &byte in text.as_bytes().iter().take(MAX_NAME_LENGTH) {
        if byte.is_ascii_alphanumeric() {
            length += 1;
        } else {
            if byte == b';' {
                length += 1;
            }
            break;
        }
    }

    // Find the longest matching named reference. Any characters consumed
    // beyond that match by MimeKit's state machine are emitted unchanged;
    // here they remain in the input for the next pass instead.
    while length > 0 {
        if let Ok(index) = NAMED_ENTITIES.binary_search_by_key(&&text[..length], |&(name, _)| name)
        {
            if in_attribute
                && is_historical_attribute_value(&text[..length], text[length..].chars().next())
            {
                return None;
            }
            return Some((length, NAMED_ENTITIES[index].1));
        }
        length -= 1;
    }
    None
}

fn is_historical_attribute_value(name: &str, next: Option<char>) -> bool {
    // In attribute values, a named reference not ending in ';' stays literal
    // if the next character after the longest match is '=' or an ASCII
    // alphanumeric. HTML retains this exception for historical compatibility.
    !name.ends_with(';') && next.is_some_and(|ch| ch == '=' || ch.is_ascii_alphanumeric())
}

fn numeric_reference(text: &str) -> Option<(usize, char)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'#') {
        return None;
    }
    let (radix, start) = if matches!(bytes.get(1), Some(b'x' | b'X')) {
        (16, 2)
    } else {
        (10, 1)
    };
    let mut index = start;
    let mut value = 0u32;
    while let Some(&byte) = bytes.get(index) {
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' if radix == 16 => u32::from(byte - b'a') + 10,
            b'A'..=b'F' if radix == 16 => u32::from(byte - b'A') + 10,
            _ => break,
        };
        // Numeric references consume every digit, no matter how many there
        // are. Saturate above the largest code point to prevent overflow
        // while still recognizing the value as out of range when decoded.
        value = (value * radix + digit).min(MAX_CODE_POINT + 1);
        index += 1;
    }
    if index == start {
        return None;
    }
    if bytes.get(index) == Some(&b';') {
        index += 1;
    }
    Some((index, numeric_value(value)))
}

fn numeric_violation(value: u32) -> Option<HtmlEntityViolation> {
    use HtmlEntityViolation::*;

    match value {
        0 => Some(NullCharacterReference),
        value if value > MAX_CODE_POINT => Some(CharacterReferenceOutsideUnicodeRange),
        0xd800..=0xdfff => Some(SurrogateCharacterReference),
        0xfdd0..=0xfdef => Some(NoncharacterCharacterReference),
        value if value & 0xfffe == 0xfffe => Some(NoncharacterCharacterReference),
        // CR is the only ASCII whitespace control that is a parse error.
        // Diagnose the original numeric value, before Windows-1252 remapping.
        0x0d | 0x01..=0x08 | 0x0b | 0x0e..=0x1f | 0x7f..=0x9f => Some(ControlCharacterReference),
        _ => None,
    }
}

// HTML numeric character reference end state.
fn numeric_value(value: u32) -> char {
    match value {
        0x00 => '\u{fffd}', // REPLACEMENT CHARACTER
        0x80 => '\u{20ac}', // EURO SIGN
        0x82 => '\u{201a}', // SINGLE LOW-9 QUOTATION MARK
        0x83 => '\u{0192}', // LATIN SMALL LETTER F WITH HOOK
        0x84 => '\u{201e}', // DOUBLE LOW-9 QUOTATION MARK
        0x85 => '\u{2026}', // HORIZONTAL ELLIPSIS
        0x86 => '\u{2020}', // DAGGER
        0x87 => '\u{2021}', // DOUBLE DAGGER
        0x88 => '\u{02c6}', // MODIFIER LETTER CIRCUMFLEX ACCENT
        0x89 => '\u{2030}', // PER MILLE SIGN
        0x8a => '\u{0160}', // LATIN CAPITAL LETTER S WITH CARON
        0x8b => '\u{2039}', // SINGLE LEFT-POINTING ANGLE QUOTATION MARK
        0x8c => '\u{0152}', // LATIN CAPITAL LIGATURE OE
        0x8e => '\u{017d}', // LATIN CAPITAL LETTER Z WITH CARON
        0x91 => '\u{2018}', // LEFT SINGLE QUOTATION MARK
        0x92 => '\u{2019}', // RIGHT SINGLE QUOTATION MARK
        0x93 => '\u{201c}', // LEFT DOUBLE QUOTATION MARK
        0x94 => '\u{201d}', // RIGHT DOUBLE QUOTATION MARK
        0x95 => '\u{2022}', // BULLET
        0x96 => '\u{2013}', // EN DASH
        0x97 => '\u{2014}', // EM DASH
        0x98 => '\u{02dc}', // SMALL TILDE
        0x99 => '\u{2122}', // TRADE MARK SIGN
        0x9a => '\u{0161}', // LATIN SMALL LETTER S WITH CARON
        0x9b => '\u{203a}', // SINGLE RIGHT-POINTING ANGLE QUOTATION MARK
        0x9c => '\u{0153}', // LATIN SMALL LIGATURE OE
        0x9e => '\u{017e}', // LATIN SMALL LETTER Z WITH CARON
        0x9f => '\u{0178}', // LATIN CAPITAL LETTER Y WITH DIAERESIS
        // Surrogates and values beyond the Unicode range are replaced.
        // Noncharacters and remaining controls are parse errors in HTML,
        // but are still emitted as-is.
        _ => char::from_u32(value).unwrap_or('\u{fffd}'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_table_is_complete_sorted_and_bounded() {
        assert_eq!(NAMED_ENTITIES.len(), 2231);
        assert_eq!(
            NAMED_ENTITIES
                .iter()
                .filter(|(name, _)| !name.ends_with(';'))
                .count(),
            106
        );
        assert!(NAMED_ENTITIES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(
            NAMED_ENTITIES.iter().map(|(name, _)| name.len()).max(),
            Some(MAX_NAME_LENGTH)
        );
        for &(name, value) in NAMED_ENTITIES {
            assert!(!name.is_empty());
            assert!(!value.is_empty());
            assert!(
                name.trim_end_matches(';')
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric())
            );
            assert_eq!(html_decode(&format!("&{name}")), value, "{name}");
            assert_eq!(
                html_decode(&format!("before&{name}!after")),
                format!("before{value}!after"),
                "{name}"
            );
        }
    }

    #[test]
    fn incremental_lookup_accepts_every_name_and_resets_between_them() {
        let mut decoder = HtmlEntityDecoder::new();
        for &(name, expected) in NAMED_ENTITIES {
            let mut prefix = String::new();
            for character in std::iter::once('&').chain(name.chars()) {
                prefix.push(character);
                assert!(decoder.push(character).unwrap(), "{name}: {character:?}");
                assert_eq!(decoder.value(), html_decode(&prefix), "{prefix}");
                assert_eq!(
                    decoder.attribute_value(None),
                    html_attribute_decode(&prefix),
                    "{prefix}"
                );
            }
            assert_eq!(decoder.value(), expected, "{name}");
            assert_eq!(decoder.attribute_value(None), expected, "{name}");
            assert_eq!(
                html_attribute_decode(&format!("&{name}")),
                expected,
                "{name}"
            );
            decoder.reset();
            assert_eq!(decoder, HtmlEntityDecoder::new());
        }
    }

    #[test]
    fn every_legacy_name_checks_all_ascii_lookahead_characters() {
        let mut decoder = HtmlEntityDecoder::new();
        for &(name, value) in NAMED_ENTITIES {
            if name.ends_with(';') {
                continue;
            }
            let reference = format!("&{name}");
            for character in reference.chars() {
                assert!(decoder.push(character).unwrap());
            }
            for byte in 0..=127u8 {
                let next = char::from(byte);
                let ambiguous = next == '=' || next.is_ascii_alphanumeric();
                assert_eq!(
                    decoder.attribute_value(Some(next)),
                    if ambiguous { reference.as_str() } else { value },
                    "{reference}, {next:?}"
                );
                // A suffix containing another ';' could complete a longer
                // name, so use '!' to terminate this legacy-reference probe.
                if next != ';' {
                    let text = format!("{reference}{next}!");
                    let expected = format!("{}{next}!", if ambiguous { &reference } else { value });
                    assert_eq!(html_attribute_decode(&text), expected, "{text:?}");
                }
            }
            decoder.reset();
        }
    }

    #[test]
    fn numeric_storage_is_bounded_and_reset_reuses_it() {
        let mut decoder = HtmlEntityDecoder::new();
        let capacity = decoder.pushed.capacity();
        assert_eq!(MAX_ENTITY_LENGTH, 33);
        for (prefix, digit, suffix, expected) in [
            ("&#", '0', "65;", "A"),
            ("&#x", '0', "41;", "A"),
            ("&#", '9', ";", "\u{fffd}"),
            ("&#x", 'F', ";", "\u{fffd}"),
        ] {
            for character in prefix.chars() {
                assert!(decoder.push(character).unwrap());
            }
            for _ in 0..100_000 {
                assert!(decoder.push(digit).unwrap());
            }
            for character in suffix.chars() {
                assert!(decoder.push(character).unwrap());
            }
            assert_eq!(decoder.value(), expected);
            assert_eq!(decoder.attribute_value(Some('=')), expected);
            assert_eq!(decoder.pushed.len(), MAX_ENTITY_LENGTH);
            assert_eq!(decoder.pushed.capacity(), capacity);
            decoder.reset();
            assert!(decoder.pushed.is_empty());
            assert_eq!(decoder.pushed.capacity(), capacity);
        }
    }

    #[test]
    fn reporting_preserves_every_named_reference_and_partial_prefix() {
        let options = HtmlDecodeOptions::default();
        for &(name, value) in NAMED_ENTITIES {
            let reference = format!("&{name}");
            let result = html_decode_with_options(&reference, &options);
            assert_eq!(result.text, value, "{name}");
            let expected = if name.ends_with(';') {
                Vec::new()
            } else {
                vec![HtmlDecodeIssue {
                    violation: HtmlEntityViolation::MissingSemicolonAfterCharacterReference,
                    offset: 0,
                }]
            };
            assert_eq!(result.issues, expected, "{name}");
            assert_eq!(
                html_attribute_decode_with_options(&reference, &options),
                result
            );
            for length in 0..=reference.len() {
                let prefix = &reference[..length];
                assert_eq!(
                    html_decode_with_options(prefix, &options).text,
                    html_decode(prefix),
                    "{prefix}"
                );
                assert_eq!(
                    html_attribute_decode_with_options(prefix, &options).text,
                    html_attribute_decode(prefix),
                    "{prefix}"
                );
            }
        }
    }
}

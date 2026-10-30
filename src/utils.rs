// Adapted from MimeKit's Utils/MimeUtils.cs and Header.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! MIME header text transformations and SMTP envelope address validation.
//!
//! Header transformations operate on complete values, not input chunks. They do not validate
//! quoted-string grammar, encode words or character sets, or make arbitrary
//! text safe to insert into headers. In particular, quoting preserves controls
//! and line breaks; unquoting tolerates malformed quotes and escapes.
//! Unfolding trims surrounding ASCII space/tab/CR/LF and removes CR/LF, but is
//! not general control-character sanitization or strict folding validation.
//! The separate SMTP validators check exact addr-spec syntax and policy without
//! rewriting the supplied text.

use std::borrow::Cow;

use crate::parse_utils::{is_whitespace_char, trim_whitespace};

pub use crate::smtp_address::{validate_smtp_address, validate_smtp_address_with_options};
pub use crate::smtp_utf8_address::{
    validate_smtp_utf8_address, validate_smtp_utf8_address_with_options,
};

/// Unfolds a header value using MimeKit's display transformation.
///
/// Trims surrounding ASCII space/tab/CR/LF, then removes all internal CR/LF,
/// including bare line breaks not followed by space or tab. All other internal
/// whitespace, quotes, and controls remain unchanged. No space is inserted in
/// place of a line break. This does not decode encoded words or character sets.
///
/// Returns a borrowed subslice when trimming alone suffices. Line-break removal
/// returns an owned string; no input is modified.
///
/// ```
/// use mimecraft::utils::unfold;
///
/// assert_eq!(unfold(" \r\nhello\r\n\tworld \r\n"), "hello\tworld");
/// assert_eq!(unfold("a\r\nb"), "ab");
/// assert_eq!(unfold("\u{a0}text\u{3000}"), "\u{a0}text\u{3000}");
/// ```
pub fn unfold(text: &str) -> Cow<'_, str> {
    let trimmed = trim_whitespace(text);
    if trimmed.bytes().any(|byte| matches!(byte, b'\r' | b'\n')) {
        let mut output = trimmed.to_owned();
        unfold_in_place(&mut output);
        Cow::Owned(output)
    } else {
        Cow::Borrowed(trimmed)
    }
}

pub(crate) fn unfold_in_place(text: &mut String) {
    text.truncate(text.trim_end_matches(is_whitespace_char).len());
    let start = text.len() - text.trim_start_matches(is_whitespace_char).len();
    text.drain(..start);
    // Remove the line breaks, not the following whitespace: collapsing spaces
    // or converting continuation tabs would change MimeKit's displayed value.
    text.retain(|ch| !matches!(ch, '\r' | '\n'));
}

/// Encloses text in double quotes and escapes backslashes and double quotes.
///
/// All other characters, including CR, LF, tabs, and NUL, are preserved.
/// This is a quoting transformation, not header-authoring validation.
///
/// ```
/// use mimecraft::utils::quote;
///
/// assert_eq!(quote("a \"quoted\" \\ path"), "\"a \\\"quoted\\\" \\\\ path\"");
/// assert_eq!(quote(""), "\"\"");
/// ```
pub fn quote(text: &str) -> String {
    let mut output = String::with_capacity(text.len() + 2);
    append_quoted(text, &mut output);
    output
}

/// Appends [`quote`]'s result without clearing existing output.
///
/// Each call writes its own opening and closing double quotes.
pub fn append_quoted(text: &str, output: &mut String) {
    output.push('"');
    for ch in text.chars() {
        if matches!(ch, '\\' | '"') {
            output.push('\\');
        }
        output.push(ch);
    }
    output.push('"');
}

/// Unquotes text using MimeKit's tolerant transformation rules.
///
/// Removes unescaped double quotes wherever they occur, removes backslash
/// quoting, and drops all CR/LF, including escaped line breaks. A final lone
/// backslash is dropped; balanced quotes are not required. Line breaks and
/// tabs reset escape state. Other characters and whitespace are preserved.
///
/// When `convert_tabs_to_spaces` is true, every tab becomes one space, even
/// when escaped. When false, tabs are preserved. Unchanged input is borrowed;
/// transformed input is owned. This performs no charset or encoded-word decoding.
///
/// ```
/// use mimecraft::utils::unquote;
///
/// assert_eq!(unquote("\"a\\\"b\r\n\tc\"", false), "a\"b\tc");
/// assert_eq!(unquote("\"a\\\"b\r\n\tc\"", true), "a\"b c");
/// assert_eq!(unquote("unbalanced\"quote\\", false), "unbalancedquote");
/// ```
pub fn unquote(text: &str, convert_tabs_to_spaces: bool) -> Cow<'_, str> {
    if !needs_unquoting(text.as_bytes(), convert_tabs_to_spaces) {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    append_unquoted(text, convert_tabs_to_spaces, &mut output);
    Cow::Owned(output)
}

/// Appends [`unquote`]'s result without clearing existing output.
///
/// The input is one complete value. Escape state is not retained between calls.
pub fn append_unquoted(text: &str, convert_tabs_to_spaces: bool, output: &mut String) {
    output.extend(unquoted(text.chars(), convert_tabs_to_spaces));
}

/// Applies [`unquote`]'s rules to raw bytes without UTF-8 or charset conversion.
///
/// Bytes other than ASCII quotes, backslashes, CR/LF, and optionally tabs are
/// preserved exactly, including invalid UTF-8 and NUL. Unchanged input is
/// borrowed; transformed input is owned. Pass a subslice to process a range.
///
/// ```
/// use mimecraft::utils::unquote_bytes;
///
/// assert_eq!(unquote_bytes(b"\"\xff\\\"\r\n\t\"", true).as_ref(), b"\xff\" ");
/// ```
pub fn unquote_bytes(input: &[u8], convert_tabs_to_spaces: bool) -> Cow<'_, [u8]> {
    if !needs_unquoting(input, convert_tabs_to_spaces) {
        return Cow::Borrowed(input);
    }
    let mut output = Vec::with_capacity(input.len());
    append_unquoted_bytes(input, convert_tabs_to_spaces, &mut output);
    Cow::Owned(output)
}

/// Appends [`unquote_bytes`]'s result without clearing existing output.
///
/// Each call handles a complete byte slice, with independent escape state.
pub fn append_unquoted_bytes(input: &[u8], convert_tabs_to_spaces: bool, output: &mut Vec<u8>) {
    // The shared transform only changes ASCII. Mapping individual bytes to
    // chars and back is lossless even for invalid UTF-8; no result exceeds 255.
    output.extend(
        unquoted(
            input.iter().copied().map(char::from),
            convert_tabs_to_spaces,
        )
        .map(|ch| ch as u8),
    );
}

fn needs_unquoting(input: &[u8], convert_tabs_to_spaces: bool) -> bool {
    input.iter().any(|&byte| {
        matches!(byte, b'\r' | b'\n' | b'\\' | b'"') || (convert_tabs_to_spaces && byte == b'\t')
    })
}

fn unquoted(
    input: impl Iterator<Item = char>,
    convert_tabs_to_spaces: bool,
) -> impl Iterator<Item = char> {
    let mut escaped = false;
    // Unescaped quotes are removed anywhere. Checking quote balance here would
    // change MimeKit's intentionally tolerant unquoting behavior.
    input.filter_map(move |ch| match ch {
        '\r' | '\n' => {
            escaped = false;
            None
        }
        '\t' => {
            escaped = false;
            Some(if convert_tabs_to_spaces { ' ' } else { '\t' })
        }
        '\\' => {
            escaped = !escaped;
            (!escaped).then_some('\\')
        }
        '"' => {
            let keep = escaped;
            escaped = false;
            keep.then_some('"')
        }
        _ => {
            escaped = false;
            Some(ch)
        }
    })
}

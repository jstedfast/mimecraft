// Adapted from MimeKit's Utils/ParseUtils.cs, Utils/ByteExtensions.cs,
// Text/UrlScanner.cs and Utils/SmtpAddressSyntax.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

pub(crate) fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

pub(crate) fn is_whitespace_char(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\r' | '\n')
}

pub(crate) fn trim_whitespace(text: &str) -> &str {
    text.trim_matches(is_whitespace_char)
}

// IPv4-address-literal = Snum 3("." Snum); each Snum is 1-3 digits, 0-255.
pub(crate) fn skip_ipv4(text: &str, index: &mut usize, allow_leading_zeros: bool) -> bool {
    for group in 0..4 {
        let start = *index;
        let mut value = 0u16;
        // Stop after four digits; more than three is invalid regardless of value.
        while *index - start < 4 && text.as_bytes().get(*index).is_some_and(u8::is_ascii_digit) {
            value = value * 10 + u16::from(text.as_bytes()[*index] - b'0');
            *index += 1;
        }
        if *index == start || *index - start > 3 || value > 255 {
            return false;
        }
        // URI dec-octets (RFC 3986) exclude leading zeros, unlike SMTP's Snum.
        if !allow_leading_zeros && *index - start > 1 && text.as_bytes()[start] == b'0' {
            return false;
        }
        if group < 3 {
            if text.as_bytes().get(*index) != Some(&b'.') {
                return false;
            }
            *index += 1;
        }
    }
    true
}

pub(crate) fn skip_ipv6(text: &str, index: &mut usize, smtp: bool) -> bool {
    // Handle full/compressed IPv6 and embedded IPv4. SMTP (RFC 5321, 4.1.3)
    // requires "::" to replace at least two groups; URI hosts (RFC 3986, 3.2.2)
    // allow one. Count groups, not colons, to reject empty single-colon groups.
    let mut compact = false;
    let mut groups = 0;
    if text.as_bytes().get(*index) == Some(&b':') {
        if text.as_bytes().get(*index + 1) != Some(&b':') {
            return false;
        }
        *index += 2;
        compact = true;
    }
    while *index < text.len() && text.as_bytes()[*index] != b']' {
        let start = *index;
        if groups == 8 {
            return false;
        }
        // Stop after five hex digits; more than four is invalid.
        while *index - start < 5
            && text
                .as_bytes()
                .get(*index)
                .is_some_and(u8::is_ascii_hexdigit)
        {
            *index += 1;
        }
        let count = *index - start;
        if count == 0 || count > 4 {
            return false;
        }
        if text.as_bytes().get(*index) == Some(&b'.') {
            // The IPv4 tail occupies two groups and must end the address.
            *index = start;
            if groups > 6 || !skip_ipv4(text, index, smtp) {
                return false;
            }
            groups += 2;
            break;
        }
        groups += 1;
        if text.as_bytes().get(*index) != Some(&b':') {
            break;
        }
        *index += 1;
        if text.as_bytes().get(*index) == Some(&b':') {
            if compact {
                return false;
            }
            compact = true;
            *index += 1;
        } else if !text
            .as_bytes()
            .get(*index)
            .is_some_and(u8::is_ascii_hexdigit)
        {
            return false;
        }
    }
    if compact {
        groups <= if smtp { 6 } else { 7 }
    } else {
        groups == 8
    }
}

pub(crate) fn skip_whitespace(input: &[u8], index: &mut usize) {
    while input.get(*index).is_some_and(|&byte| is_whitespace(byte)) {
        *index += 1;
    }
}

pub(crate) fn is_atom(byte: u8) -> bool {
    byte >= 128 || byte.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&byte)
}

pub(crate) fn is_ascii_atom(byte: u8) -> bool {
    byte.is_ascii() && is_atom(byte)
}

pub(crate) fn is_media_type(byte: u8) -> bool {
    is_ascii_atom(byte) && byte != b'/'
}

pub(crate) fn is_media_subtype(byte: u8) -> bool {
    is_token(byte) || byte == b'/'
}

pub(crate) fn is_token(byte: u8) -> bool {
    (33..127).contains(&byte) && !b"()<>@,;:\\\"/[]?=".contains(&byte)
}

pub(crate) fn is_attribute(byte: u8) -> bool {
    is_token(byte) && !b"*'%".contains(&byte)
}

/// Skips a quoted string starting at its opening quote. On failure, returns
/// that quote's offset and leaves the index at the end of the input.
pub(crate) fn skip_quoted(input: &[u8], index: &mut usize) -> Result<(), usize> {
    let start = *index;
    *index += 1;
    while *index < input.len() {
        let byte = input[*index];
        *index += 1;
        match byte {
            b'\\' => {
                if *index < input.len() {
                    *index += 1;
                }
            }
            b'"' => return Ok(()),
            _ => {}
        }
    }
    Err(start)
}

/// Skips MIME whitespace and nested comments, returning the byte offset of an
/// unterminated comment. The caller supplies a valid byte index into the input.
pub(crate) fn skip_comments_and_whitespace(input: &[u8], index: &mut usize) -> Result<(), usize> {
    loop {
        skip_whitespace(input, index);
        if input.get(*index) != Some(&b'(') {
            return Ok(());
        }

        skip_comment(input, index)?;
    }
}

/// Skips one nested comment, returning its opening offset if it is unclosed.
pub(crate) fn skip_comment(input: &[u8], index: &mut usize) -> Result<(), usize> {
    let start = *index;
    let mut depth = 1usize;
    *index += 1;
    while *index < input.len() && depth > 0 {
        let byte = input[*index];
        *index += 1;
        match byte {
            // A quoted pair cannot open or close a comment. Skipping both
            // bytes also handles alternating escaped and unescaped slashes.
            b'\\' => {
                if *index < input.len() {
                    *index += 1;
                }
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 { Err(start) } else { Ok(()) }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DomainParseErrorKind {
    InvalidDomain,
    IncompleteLiteral,
    IncompleteComment,
    InvalidUtf8,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DomainParseError {
    pub kind: DomainParseErrorKind,
    pub offset: usize,
}

fn domain_cfws(input: &[u8], index: &mut usize) -> Result<(), DomainParseError> {
    skip_comments_and_whitespace(input, index).map_err(|offset| DomainParseError {
        kind: DomainParseErrorKind::IncompleteComment,
        offset,
    })
}

/// Appends a normalized domain, retaining cursor progress on failure.
///
/// Sentinels are the enclosing grammar's delimiters after a tolerated trailing
/// dot. They do not otherwise restrict which atom characters the domain accepts.
pub(crate) fn parse_domain(
    input: &[u8],
    index: &mut usize,
    sentinels: &[u8],
    output: &mut String,
) -> Result<(), DomainParseError> {
    if input.get(*index) == Some(&b'[') {
        return parse_domain_literal(input, index, output);
    }

    loop {
        let start = *index;
        while input.get(*index).is_some_and(|&byte| is_atom(byte)) {
            *index += 1;
        }
        if *index == start {
            return Err(DomainParseError {
                kind: DomainParseErrorKind::InvalidDomain,
                offset: start,
            });
        }
        let atom =
            std::str::from_utf8(&input[start..*index]).map_err(|error| DomainParseError {
                kind: DomainParseErrorKind::InvalidUtf8,
                offset: start + error.valid_up_to(),
            })?;
        output.push_str(atom);

        let comment = *index;
        domain_cfws(input, index)?;
        if input.get(*index) != Some(&b'.') {
            // Only absorb comments/whitespace here if another domain label
            // follows. The enclosing parser handles trailing CFWS.
            *index = comment;
            break;
        }
        *index += 1;
        domain_cfws(input, index)?;
        // Allow domains to end with a '.', but strip it off.
        if *index == input.len() || sentinels.contains(&input[*index]) {
            break;
        }
        output.push('.');
    }
    Ok(())
}

fn parse_domain_literal(
    input: &[u8],
    index: &mut usize,
    output: &mut String,
) -> Result<(), DomainParseError> {
    let start = *index;
    *index += 1;
    output.push('[');
    loop {
        skip_whitespace(input, index);
        let Some(&byte) = input.get(*index) else {
            return Err(DomainParseError {
                kind: DomainParseErrorKind::IncompleteLiteral,
                offset: start,
            });
        };
        if byte == b']' {
            output.push(']');
            *index += 1;
            return Ok(());
        }
        // MimeKit accepts printable ASCII except '[', '\\', and ']'; this
        // is token parsing, not IP-address validation.
        if !(33..=126).contains(&byte) || matches!(byte, b'[' | b'\\') {
            return Err(DomainParseError {
                kind: DomainParseErrorKind::InvalidDomain,
                offset: *index,
            });
        }
        output.push(char::from(byte));
        *index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{is_whitespace_char, trim_whitespace};

    #[test]
    fn trims_only_mime_whitespace() {
        for (text, expected) in [
            ("", ""),
            (" \t\r\n", ""),
            (" \tvalue\r\n ", "value"),
            (" \t\u{a0}value\u{2008}\r\n ", "\u{a0}value\u{2008}"),
            ("\x0bvalue\x0c", "\x0bvalue\x0c"),
        ] {
            assert_eq!(trim_whitespace(text), expected);
        }
    }

    #[test]
    fn recognizes_exactly_four_whitespace_characters() {
        for ch in '\0'..=char::MAX {
            assert_eq!(
                is_whitespace_char(ch),
                [' ', '\t', '\r', '\n'].contains(&ch),
                "U+{:04X}",
                u32::from(ch),
            );
        }
    }
}

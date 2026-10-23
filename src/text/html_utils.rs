// Adapted from MimeKit's Text/HtmlUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

/// The delimiter used around an encoded HTML attribute value.
///
/// Both encoders always escape ampersands and angle brackets. Attribute
/// encoding escapes only the selected quote; the other quote remains literal.
/// This type excludes unquoted attributes and invalid quote characters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HtmlAttributeQuote {
    /// Wraps the value in apostrophes, escaping embedded apostrophes as `&#39;`.
    Single,
    /// Wraps the value in double quotes, escaping them as `&quot;`.
    #[default]
    Double,
}

impl HtmlAttributeQuote {
    fn character(self) -> char {
        match self {
            Self::Single => '\'',
            Self::Double => '"',
        }
    }
}

pub(super) fn is_valid_attribute_name(name: &str) -> bool {
    // HTML tokenizer attribute-name state: reject delimiters, not all
    // characters outside a stricter XML-style name grammar.
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| !b"\0\t\r\n\x0c /=>\"'<".contains(&byte))
}

pub(super) fn is_valid_tag_name(name: &str) -> bool {
    // The tag-name state has a smaller delimiter set than attribute names.
    !name.is_empty() && name.bytes().all(|byte| !b"\0\t\r\n\x0c />".contains(&byte))
}

/// Encodes plain text as HTML character data using MimeKit's escaping rules.
///
/// Escapes `&`, `<`, `>`, apostrophes, and double quotes. Preserves tab, CR, LF,
/// and form feed, but drops other C0 controls and U+007F through U+009F.
/// Characters from U+00A0 onward become decimal numeric character references,
/// including supplementary characters as one reference per Unicode scalar.
/// Rust strings cannot contain the unpaired UTF-16 surrogates accepted by
/// MimeKit. The result is ASCII, without newline normalization or added markup.
///
/// Existing entity references are escaped, not recognized or decoded.
/// This is character-data encoding, not HTML sanitization, JavaScript/CSS
/// escaping, URL validation, or conversion into an HTML document.
///
/// ```
/// use mimecraft::text::html_encode;
///
/// assert_eq!(html_encode("<b>caf\u{e9}</b>"), "&lt;b&gt;caf&#233;&lt;/b&gt;");
/// assert_eq!(html_encode("&amp;"), "&amp;amp;");
/// ```
pub fn html_encode(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    encode_into(&mut output, text, None);
    output
}

/// Encodes and quotes an HTML attribute value using MimeKit's escaping rules.
///
/// Includes both surrounding quote characters, even for empty input. Escapes
/// the selected quote, `&`, `<`, and `>`; leaves the other quote literal.
/// Control removal, whitespace preservation, and numeric character references
/// follow [`html_encode`]. Pass [`HtmlAttributeQuote::Double`] (or `Default`)
/// for MimeKit's default quoting.
///
/// Does not validate attribute names or values. In particular, encoding a
/// value does not make a URL scheme, event handler, or CSS declaration safe.
/// No HTML parsing, entity decoding, or stream I/O occurs.
///
/// ```
/// use mimecraft::text::{HtmlAttributeQuote, html_attribute_encode};
///
/// assert_eq!(
///     html_attribute_encode("a \"quote\" & 'another'", HtmlAttributeQuote::Double),
///     "\"a &quot;quote&quot; &amp; 'another'\""
/// );
/// assert_eq!(
///     html_attribute_encode("it's", HtmlAttributeQuote::Single),
///     "'it&#39;s'"
/// );
/// ```
pub fn html_attribute_encode(value: &str, quote: HtmlAttributeQuote) -> String {
    let mut output = String::with_capacity(value.len());
    output.push(quote.character());
    encode_into(&mut output, value, Some(quote));
    output.push(quote.character());
    output
}

pub(super) fn encode_into(output: &mut String, text: &str, quote: Option<HtmlAttributeQuote>) {
    // Copy the unchanged prefix in one operation, using UTF-8 byte offsets.
    let index = text
        .find(|character: char| match character {
            '\t' | '\r' | '\n' | '\u{c}' => false,
            '&' | '<' | '>' => true,
            '\'' => quote != Some(HtmlAttributeQuote::Double),
            '"' => quote != Some(HtmlAttributeQuote::Single),
            _ => !(' '..'\u{7f}').contains(&character),
        })
        .unwrap_or(text.len());
    output.push_str(&text[..index]);

    for character in text[index..].chars() {
        match character {
            '\t' | '\r' | '\n' | '\u{c}' => output.push(character),
            '\'' if quote != Some(HtmlAttributeQuote::Double) => output.push_str("&#39;"),
            '"' if quote != Some(HtmlAttributeQuote::Single) => output.push_str("&quot;"),
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\0'..='\u{1f}' | '\u{7f}'..='\u{9f}' => {
                // Drop illegal control characters, as in MimeKit.
            }
            '\u{a0}'.. => {
                // Latin-1 and other non-ASCII scalars become numeric references.
                // Rust chars already combine what would be a UTF-16 surrogate pair.
                write_numeric_character_reference(output, character);
            }
            _ => {
                // SPACE and other printable (safe) ASCII.
                output.push(character);
            }
        }
    }
}

fn write_numeric_character_reference(output: &mut String, character: char) {
    // Write a decimal reference without allocating a string per character.
    // U+10FFFF = 1114111 needs seven digits, plus "&#" and ";".
    let mut buffer = [0u8; 10];
    let mut index = buffer.len() - 1;
    let mut value = u32::from(character);
    buffer[index] = b';';
    loop {
        index -= 1;
        buffer[index] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    index -= 1;
    buffer[index] = b'#';
    index -= 1;
    buffer[index] = b'&';
    output.push_str(std::str::from_utf8(&buffer[index..]).expect("reference contains only ASCII"));
}

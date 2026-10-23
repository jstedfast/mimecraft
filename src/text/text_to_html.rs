// Adapted from MimeKit's Text/TextToHtml.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    HeaderFooterFormat, HtmlAttributeQuote, html_utils::encode_into, next_line, unquote,
    url_scanner,
};

/// Converts decoded plain text into HTML, with automatic links and quotations.
///
/// By default, wraps the result in `<html><body>` and `</body></html>`.
/// Each logical input line produces `<br/>`, including an unterminated final
/// line. Empty input produces an empty body. CRLF, LF, and bare CR are accepted.
/// Leading `>` markers, optionally separated by one ASCII space each, become
/// nested `<blockquote>` elements rather than literal text.
///
/// Text and link attributes use [`html_encode`](super::html_encode)'s character
/// and control handling. Whitespace is not expanded into nonbreaking spaces;
/// browsers can collapse runs of spaces and tabs. Input markup is escaped.
///
/// Automatic links follow MimeKit's heuristic scanner, not a general URL or
/// email-address validator. Supported prefixes are `http://`, `https://`,
/// `ftp://`, `sftp://`, `file://`, `news://`, `nntp://`, `telnet://`, `webcal://`,
/// `callto:`, `h323:`, `sip:`, `mailto:`, `www.`, and `ftp.`. Bare addresses
/// become `mailto:` links; `www.` and `ftp.` receive their implied schemes.
/// Prefix matching ignores ASCII case. Bare address domains need at least two
/// labels and a final label of at least two Unicode letters or a validated
/// IDNA A-label (`xn--...`), using the existing [`crate::idna`] helpers.
///
/// Unquoted link components exclude Unicode whitespace, controls, and format
/// characters. Local parts, domain labels, and complete domains are limited to
/// 64, 63, and 255 UTF-8 bytes respectively, rather than MimeKit's UTF-16 units.
/// Trailing punctuation and enclosing prose delimiters are excluded; balanced
/// inner brackets in paths, queries, and fragments are retained. Queries and
/// fragments may immediately follow the host, including an empty `#`.
/// Schemes ending in `://` support userinfo and bracketed IPv6 hosts. Address
/// literals follow SMTP syntax; URI IPv6 hosts use URI compression and embedded
/// IPv4 rules instead. IPvFuture and scoped IPv6 hosts are not recognized.
/// Source heuristics such as partial-word prefix matches and unrestricted
/// numeric ports remain. Targets are not canonicalized or checked for trust.
///
/// No MIME modification, charset conversion, stream I/O, or tag callbacks occur.
/// This is not an HTML sanitizer: HTML-format headers and footers are trusted
/// caller-provided markup and are emitted verbatim.
///
/// ```
/// use mimecraft::text::TextToHtml;
///
/// let converter = TextToHtml {
///     output_html_fragment: true,
///     ..TextToHtml::new()
/// };
/// assert_eq!(converter.convert("> Hello & goodbye"),
///            "<blockquote>Hello &amp; goodbye<br/></blockquote>");
/// assert_eq!(converter.convert("Visit www.example.com"),
///            "Visit <a href=\"http://www.example.com\">www.example.com</a><br/>");
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextToHtml {
    /// Omits the outer HTML/body tags when true; defaults to false.
    pub output_html_fragment: bool,
    /// Optional content before the converted body; defaults to `None`.
    pub header: Option<String>,
    /// How to interpret [`header`](Self::header); defaults to plain text.
    pub header_format: HeaderFooterFormat,
    /// Optional content after the converted body; defaults to `None`.
    pub footer: Option<String>,
    /// How to interpret [`footer`](Self::footer); defaults to plain text.
    pub footer_format: HeaderFooterFormat,
}

impl TextToHtml {
    /// Creates a converter producing a complete document without decorations.
    pub fn new() -> Self {
        Self::default()
    }

    /// Converts a complete Unicode string without changing the converter.
    pub fn convert(&self, text: &str) -> String {
        let mut output = String::new();
        if !self.output_html_fragment {
            output.push_str("<html><body>");
        }
        write_decoration(&mut output, self.header.as_deref(), self.header_format);
        write_fragment(&mut output, text);
        write_decoration(&mut output, self.footer.as_deref(), self.footer_format);
        if !self.output_html_fragment {
            output.push_str("</body></html>");
        }
        output
    }
}

pub(super) fn write_decoration(
    output: &mut String,
    text: Option<&str>,
    format: HeaderFooterFormat,
) {
    if let Some(text) = text {
        match format {
            HeaderFooterFormat::Text => write_fragment(output, text),
            HeaderFooterFormat::Html => output.push_str(text),
        }
    }
}

fn write_fragment(output: &mut String, mut text: &str) {
    let mut current_depth = 0;
    while let Some(line) = next_line(&mut text) {
        let (line, depth) = unquote(line);
        write_quote_depth(output, &mut current_depth, depth);
        write_text(output, line);
        output.push_str("<br/>");
    }
    // Finish each fragment's quotation before writing the next decoration.
    write_quote_depth(output, &mut current_depth, 0);
}

pub(super) fn write_quote_depth(output: &mut String, current_depth: &mut usize, depth: usize) {
    while *current_depth < depth {
        output.push_str("<blockquote>");
        *current_depth += 1;
    }
    while depth < *current_depth {
        output.push_str("</blockquote>");
        *current_depth -= 1;
    }
}

pub(super) fn write_text(output: &mut String, mut text: &str) {
    while let Some(found) = url_scanner::scan(text) {
        // Write everything up to the match, then encode its target and label
        // separately. An address's quoted local part must not end the attribute.
        encode_into(output, &text[..found.start], None);
        let link = &text[found.start..found.end];
        output.push_str("<a href=\"");
        output.push_str(found.prefix);
        encode_into(output, link, Some(HtmlAttributeQuote::Double));
        output.push_str("\">");
        encode_into(output, link, None);
        output.push_str("</a>");
        text = &text[found.end..];
    }
    encode_into(output, text, None);
}

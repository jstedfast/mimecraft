// Adapted from MimeKit's Text/FlowedToHtml.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    HeaderFooterFormat, next_line, output_newline,
    text_to_html::{write_decoration, write_quote_depth, write_text},
    unquote_flowed,
};
use crate::encodings::{EncoderOptionsError, NewLineFormat};

/// Converts decoded flowed text into HTML paragraphs, links, and quotations.
///
/// Removes one stuffing space after a contiguous run of `>` quote markers.
/// Lines ending in an ASCII space continue the paragraph; quote-depth changes
/// take precedence over soft breaks. Tabs and Unicode whitespace are not soft
/// breaks. CRLF, LF, and bare CR input line endings are accepted.
///
/// Nonempty paragraphs become `<p>...</p>`, empty hard paragraphs become
/// `<br/>`, and quotation levels become nested `<blockquote>` elements.
/// Every emitted paragraph has a markup newline, including pending nonempty
/// flowed text at EOF. Empty pending paragraphs are omitted, even on a quote
/// depth change. Like MimeKit, `-- ` is treated as a soft break rather than
/// special-cased as a signature separator; this is not strict RFC 3676 validation.
///
/// Uses [`TextToHtml`](super::TextToHtml)'s escaping and automatic-link rules
/// after joining each paragraph. Text-format headers and footers use plain
/// text conversion, not flowed conversion, and close their own quote blocks.
/// HTML-format decorations are trusted markup copied verbatim, not sanitized.
///
/// Output defaults to a complete HTML document and CRLF paragraph newlines,
/// independent of the platform. No MIME modification, charset decoding,
/// stream I/O, or tag callbacks occur. Each conversion starts fresh.
///
/// ```
/// use mimecraft::text::FlowedToHtml;
///
/// let mut converter = FlowedToHtml::new();
/// converter.output_html_fragment = true;
/// assert_eq!(converter.convert("> hello \n> world"),
///            "<blockquote><p>hello world</p>\r\n</blockquote>");
/// converter.delete_space = true;
/// assert_eq!(converter.convert("soft \nbreak"), "<p>softbreak</p>\r\n");
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FlowedToHtml {
    /// Removes exactly one trailing soft-break space when true.
    ///
    /// Defaults to false. For MIME text, enable only when `delsp=yes`, ignoring
    /// ASCII case. Other trailing spaces and non-ASCII whitespace are preserved.
    pub delete_space: bool,
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
    new_line_format: NewLineFormat,
}

impl FlowedToHtml {
    /// Creates a complete-document converter with CRLF paragraph newlines.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a converter with explicit paragraph markup line endings.
    ///
    /// This does not rewrite trusted HTML decorations or insert newlines
    /// between the document wrappers, decorations, or quotation tags.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for `Mixed`, which
    /// describes observed input rather than a valid output format.
    pub fn with_new_line_format(
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        if new_line_format == NewLineFormat::Mixed {
            return Err(EncoderOptionsError::InvalidNewLineFormat(new_line_format));
        }
        Ok(Self {
            new_line_format,
            ..Self::default()
        })
    }

    /// Returns the configured paragraph markup line endings.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Converts a complete Unicode string without changing the converter.
    pub fn convert(&self, text: &str) -> String {
        let mut output = String::new();
        if !self.output_html_fragment {
            output.push_str("<html><body>");
        }
        write_decoration(&mut output, self.header.as_deref(), self.header_format);
        self.write_fragment(&mut output, text);
        write_decoration(&mut output, self.footer.as_deref(), self.footer_format);
        if !self.output_html_fragment {
            output.push_str("</body></html>");
        }
        output
    }

    fn write_fragment(&self, output: &mut String, mut text: &str) {
        let newline = output_newline(self.new_line_format);
        let mut current_depth = 0;
        let mut paragraph_depth = 0;
        let mut paragraph = String::new();
        while let Some(line) = next_line(&mut text) {
            let (unquoted, depth) = unquote_flowed(line);
            if paragraph.is_empty() {
                paragraph_depth = depth;
            } else if depth != paragraph_depth {
                // RFC 3676: if a folded line changes quote depth, quote-depth
                // rules win over the soft break and start a new paragraph.
                write_paragraph(
                    output,
                    &paragraph,
                    paragraph_depth,
                    &mut current_depth,
                    newline,
                );
                paragraph_depth = depth;
                paragraph.clear();
            }
            paragraph.push_str(unquoted);
            if !unquoted.ends_with(' ') {
                // The line did not end with a space, so the next line starts
                // a new paragraph.
                write_paragraph(
                    output,
                    &paragraph,
                    paragraph_depth,
                    &mut current_depth,
                    newline,
                );
                paragraph_depth = 0;
                paragraph.clear();
            } else if self.delete_space {
                // A trailing space marks the next line as a continuation;
                // delsp removes exactly that marker before joining.
                paragraph.pop();
            }
        }
        // Flush remaining paragraph text, even when the last line was flowed
        // (MimeKit issue #1130). Unlike FlowedToText, HTML still ends the paragraph.
        if !paragraph.is_empty() {
            write_paragraph(
                output,
                &paragraph,
                paragraph_depth,
                &mut current_depth,
                newline,
            );
        }
        write_quote_depth(output, &mut current_depth, 0);
    }
}

fn write_paragraph(
    output: &mut String,
    paragraph: &str,
    depth: usize,
    current_depth: &mut usize,
    newline: &str,
) {
    write_quote_depth(output, current_depth, depth);
    if paragraph.is_empty() {
        output.push_str("<br/>");
    } else {
        output.push_str("<p>");
        write_text(output, paragraph);
        output.push_str("</p>");
    }
    output.push_str(newline);
}

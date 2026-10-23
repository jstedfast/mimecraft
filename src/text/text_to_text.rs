// Adapted from MimeKit's Text/TextToText.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

/// Copies plain text verbatim, optionally prepending a header and appending a footer.
///
/// Preserves all characters, whitespace, and line endings, including an
/// unterminated final line. No separators are inserted between the header,
/// input, and footer; callers must include any desired spaces or newlines.
/// Absent and empty decorations have the same effect. Decorations are emitted
/// even when the input is empty.
///
/// Like the other whole-string converters, this operates on decoded Unicode
/// text. It performs no charset conversion, stream I/O, escaping, normalization,
/// or MIME header/body modification. The header and footer are text, not MIME
/// message headers or multipart delimiters.
///
/// ```
/// use mimecraft::text::TextToText;
///
/// let converter = TextToText {
///     header: Some("Header\r\n".into()),
///     footer: Some("\r\nFooter".into()),
/// };
/// assert_eq!(converter.convert("body"), "Header\r\nbody\r\nFooter");
/// assert_eq!(TextToText::new().convert("body"), "body");
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextToText {
    /// Text prepended verbatim to every conversion; defaults to `None`.
    pub header: Option<String>,
    /// Text appended verbatim to every conversion; defaults to `None`.
    pub footer: Option<String>,
}

impl TextToText {
    /// Creates a converter without a header or footer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Copies a complete Unicode string without changing the converter.
    pub fn convert(&self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        if let Some(header) = &self.header {
            output.push_str(header);
        }

        // Copy directly rather than scanning logical lines: even a final bare
        // CR and the boundaries between decorations and body stay unchanged.
        output.push_str(text);

        if let Some(footer) = &self.footer {
            output.push_str(footer);
        }
        output
    }
}

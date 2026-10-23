// Adapted from MimeKit's Text/HeaderFooterFormat.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

/// The format of a [`TextToHtml`](super::TextToHtml) or
/// [`FlowedToHtml`](super::FlowedToHtml) header or footer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HeaderFooterFormat {
    /// Plain text converted to an HTML fragment, including links and quotations.
    #[default]
    Text,
    /// Trusted HTML copied verbatim, without escaping or sanitization.
    Html,
}

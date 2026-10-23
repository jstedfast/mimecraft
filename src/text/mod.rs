// Adapted from MimeKit's Text/FlowedToText.cs and Text/TextToFlowed.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Text-format conversion and HTML helpers, separate from charset/transfer decoding.
//!
//! Provides verbatim plain-text copying with optional headers and footers,
//! flowed-text wrapping and unwrapping, plain-text previews, and HTML
//! text/attribute encoding, whole-string and incremental character-reference
//! decoding with opt-in reference diagnostics, and plain/flowed HTML rendering,
//! plus an in-memory HTML writer, known tag/attribute identifiers, checked
//! attribute collections, and owned HTML token serialization.
//! HTML tokenization, HTML-input conversion, rich-text conversion, and streaming
//! converters are deferred.

mod flowed_to_html;
mod flowed_to_text;
mod header_footer_format;
mod html_attribute;
mod html_attribute_collection;
mod html_entities;
mod html_entity_decoder;
mod html_ids;
mod html_token;
mod html_utils;
mod html_writer;
mod plain_text_previewer;
mod text_to_flowed;
mod text_to_html;
mod text_to_text;
mod url_scanner;

use crate::encodings::NewLineFormat;

pub use flowed_to_html::FlowedToHtml;
pub use flowed_to_text::FlowedToText;
pub use header_footer_format::HeaderFooterFormat;
pub use html_attribute::{HtmlAttribute, HtmlAttributeError};
pub use html_attribute_collection::HtmlAttributeCollection;
pub use html_entity_decoder::{
    DecodedHtml, HtmlDecodeIssue, HtmlDecodeOptions, HtmlEntityDecoder, HtmlEntityDecoderError,
    HtmlEntityViolation, html_attribute_decode, html_attribute_decode_with_options, html_decode,
    html_decode_with_options,
};
pub use html_ids::{HtmlAttributeId, HtmlTagId};
pub use html_token::{
    HtmlCommentToken, HtmlDataToken, HtmlDocTypeError, HtmlDocTypeToken, HtmlTagToken, HtmlToken,
    HtmlTokenKind,
};
pub use html_utils::{HtmlAttributeQuote, html_attribute_encode, html_encode};
pub use html_writer::{HtmlWriter, HtmlWriterError, HtmlWriterState};
pub use plain_text_previewer::{InvalidPreviewLength, PlainTextPreviewer};
pub use text_to_flowed::TextToFlowed;
pub use text_to_html::TextToHtml;
pub use text_to_text::TextToText;

fn unquote(mut line: &str) -> (&str, usize) {
    let mut depth = 0;
    while let Some(unquoted) = line.strip_prefix('>') {
        depth += 1;
        line = unquoted.strip_prefix(' ').unwrap_or(unquoted);
    }
    (line, depth)
}

fn unquote_flowed(line: &str) -> (&str, usize) {
    let depth = line.bytes().take_while(|&byte| byte == b'>').count();
    let unquoted = &line[depth..];
    // A leading space after the contiguous quote markers is stuffing, not text.
    (unquoted.strip_prefix(' ').unwrap_or(unquoted), depth)
}

fn output_newline(format: NewLineFormat) -> &'static str {
    match format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => unreachable!("constructors reject mixed output newlines"),
    }
}

fn next_line<'a>(remaining: &mut &'a str) -> Option<&'a str> {
    if remaining.is_empty() {
        return None;
    }
    if let Some(index) = remaining.find(['\r', '\n']) {
        let line = &remaining[..index];
        let length = if remaining[index..].starts_with("\r\n") {
            2
        } else {
            1
        };
        *remaining = &remaining[index + length..];
        Some(line)
    } else {
        Some(std::mem::take(remaining))
    }
}

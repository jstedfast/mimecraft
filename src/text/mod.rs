// Adapted from MimeKit's Text/FlowedToText.cs and Text/TextToFlowed.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Text-format conversion and HTML helpers, separate from charset/transfer decoding.
//!
//! Provides verbatim plain-text copying with optional headers and footers,
//! flowed-text wrapping and unwrapping, string and streaming UTF-8 plain-text
//! previews, and HTML text/attribute encoding, whole-string and incremental character-reference
//! decoding with opt-in reference diagnostics, and plain/flowed HTML rendering,
//! plus an in-memory HTML writer, known tag/attribute identifiers, checked
//! attribute collections, and owned HTML token serialization.
//! A separate token-consuming HTML structural controller tracks namespace
//! transitions, implicit closures, and lexical content-mode selection without
//! parsing HTML text or constructing a DOM.
//! A sans-I/O UTF-8 HTML tokenizer uses that controller to select lexical states,
//! preserving partial input across chunks. A synchronous `BufRead` adapter
//! drives the same engine without embedding I/O in the scanner.
//! Enriched-text HTML conversion adds character and paragraph formatting,
//! validated parameters, malformed-command recovery, and explicit resource limits.
//! A byte-oriented streaming RTF tokenizer supplies owned tokens, keyword
//! classifications, positions, and optionally capped lexical recovery callbacks.
//! An RTF validator adds document-header and group-structure checks without
//! changing tokens or interpreting their formatting.
//! RTF-to-text conversion adds bounded group/font state, charset and Unicode
//! decoding, destination suppression, layout, and optional diagnostics.
//! RTF-to-HTML rendering shares that interpreter and adds character formatting,
//! paragraph alignment, colors, tables, and bounded hyperlink-field collection.
//! Tag callbacks can customize generated plain-text, flowed-text, and RTF body
//! tags, replace attributes, request end-tag callbacks, and suppress inner content.
//! A separate raw RTF HTML extractor recovers unfiltered encapsulated markup
//! without changing either RTF renderer's behavior.
//! Decoded RTF hyperlink-field instructions can be parsed without allocating.
//! HTML-to-text conversion preserves structural formatting with bounded UTF-8
//! input/output, optional link and image text, and a synchronous writer adapter.
//! HTML body previews add MimeKit's whitespace, image and list heuristics with
//! scalar-safe truncation and constant-time open-element tracking.
//! HTML-to-HTML rewriting exposes original tags to callbacks and supports
//! fragments, comment filtering, and renderer-aware noscript handling.
//! Async reader adapters and other streaming-output converters are deferred.

mod enriched_to_html;
mod flowed_to_html;
mod flowed_to_text;
mod header_footer_format;
mod html_attribute;
mod html_attribute_collection;
mod html_entities;
mod html_entity_decoder;
mod html_ids;
mod html_namespace;
mod html_open_element_stack;
mod html_string;
mod html_tag_context;
mod html_text_previewer;
mod html_to_html;
mod html_to_text;
mod html_token;
mod html_tokenizer;
mod html_tokenizer_reader;
mod html_tree_construction;
mod html_utils;
mod html_writer;
mod plain_text_previewer;
mod rtf_html_extractor;
mod rtf_hyperlink;
mod rtf_interpreter;
mod rtf_keyword;
mod rtf_reporting;
mod rtf_to_html;
mod rtf_to_text;
mod rtf_tokenizer;
mod rtf_validator;
mod text_html_callbacks;
mod text_to_flowed;
mod text_to_html;
mod text_to_text;
mod url_scanner;

use crate::encodings::NewLineFormat;

pub use enriched_to_html::{EnrichedToHtml, EnrichedToHtmlError};
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
pub use html_namespace::HtmlNamespace;
pub use html_tag_context::HtmlTagContext;
pub use html_text_previewer::HtmlTextPreviewer;
pub use html_to_html::{HtmlNoScriptHandling, HtmlToHtml};
pub use html_to_text::{HtmlToText, HtmlToTextError};
pub use html_token::{
    HtmlCommentToken, HtmlDataToken, HtmlDocTypeError, HtmlDocTypeToken, HtmlTagToken, HtmlToken,
    HtmlTokenKind,
};
pub use html_tokenizer::{
    HtmlSourcePosition, HtmlTokenizer, HtmlTokenizerError, HtmlTokenizerOptions,
    HtmlTokenizerResult,
};
pub use html_tokenizer_reader::HtmlTokenizerReader;
pub use html_tree_construction::{
    HtmlClosedElement, HtmlContentMode, HtmlTagDisposition, HtmlTagResult, HtmlTreeConstruction,
    InvalidHtmlElementDepth,
};
pub use html_utils::{HtmlAttributeQuote, html_attribute_encode, html_encode};
pub use html_writer::{HtmlWriter, HtmlWriterError, HtmlWriterState};
pub use plain_text_previewer::{InvalidPreviewLength, PlainTextPreviewer};
pub use rtf_html_extractor::RtfHtmlExtractor;
pub use rtf_hyperlink::parse_rtf_hyperlink;
pub use rtf_keyword::RtfKeyword;
pub use rtf_to_html::RtfToHtml;
pub use rtf_to_text::RtfToText;
pub use rtf_tokenizer::{
    RtfControlWord, RtfLexicalIssue, RtfLexicalViolation, RtfSourcePosition, RtfToken,
    RtfTokenKind, RtfTokenizer, RtfTokenizerOptions,
};
pub use rtf_validator::{RtfIssue, RtfValidationOptions, RtfValidator, RtfViolation};
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

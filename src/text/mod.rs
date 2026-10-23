// Adapted from MimeKit's Text/FlowedToText.cs and Text/TextToFlowed.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Whole-string text-format conversion, separate from charset/transfer decoding.
//!
//! Provides flowed-text wrapping and unwrapping. HTML rendering, rich-text
//! conversion, and streaming converters are deferred.

mod flowed_to_text;
mod text_to_flowed;

use crate::encodings::NewLineFormat;

pub use flowed_to_text::FlowedToText;
pub use text_to_flowed::TextToFlowed;

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

// Adapted from MimeKit's TextPart.cs and Text/TextEncodingConfidence.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use crate::{
    ContentType,
    charsets::{Charset, bom_charset},
};

use super::{HtmlAttributeId, HtmlTagId, HtmlToken, HtmlTokenizerReader};

/// Confidence in a charset selected from MIME or HTML metadata.
///
/// Failed detection is represented by `None`, not an Undefined variant.
/// MimeKit's Irrelevant state for null content has no equivalent: Rust MIME
/// parts always have a stream, including when that stream is empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEncodingConfidence {
    /// An HTML meta declaration; not a validation of the body bytes.
    Tentative,
    /// A supported MIME declaration or a Unicode byte-order mark.
    Certain,
}

pub(crate) struct DetectedEncoding {
    pub charset: Charset,
    pub confidence: TextEncodingConfidence,
    // MimeKit's BOM/default UTF-8 and UTF-16-meta coercion use its exception
    // decoder; ordinary charset lookups use the codec's replacement decoder.
    pub strict_utf8: bool,
}

pub(crate) fn declared_encoding(content_type: Option<&ContentType>) -> Option<DetectedEncoding> {
    // If the transport layer specifies a supported character encoding,
    // return that encoding with confidence certain.
    let charset = Charset::from_label(content_type?.charset()?).ok()?;
    Some(DetectedEncoding {
        charset,
        confidence: TextEncodingConfidence::Certain,
        strict_utf8: false,
    })
}

pub(crate) fn detect_encoding(
    content_type: Option<&ContentType>,
    bytes: &[u8],
) -> Option<DetectedEncoding> {
    if let Some(detected) = declared_encoding(content_type) {
        return Some(detected);
    }
    if let Some(charset) = bom_charset(bytes) {
        return Some(DetectedEncoding {
            charset,
            confidence: TextEncodingConfidence::Certain,
            strict_utf8: charset == Charset::UTF8,
        });
    }
    if content_type.is_some_and(|value| value.is_mime_type("text", "html")) {
        // Limit processing to the first 1024 transfer-decoded bytes, as in
        // the HTML input-byte-stream prescan. Latin-1 maps every input byte
        // without dropping malformed UTF-8 before the charset is known.
        // https://html.spec.whatwg.org/multipage/parsing.html#the-input-byte-stream
        return detect_html_encoding(&bytes[..bytes.len().min(1024)]);
    }
    None
}

fn detect_html_encoding(bytes: &[u8]) -> Option<DetectedEncoding> {
    #[derive(PartialEq)]
    enum State {
        None,
        Html,
        Head,
    }
    let text = Charset::LATIN1.decode_lossy(bytes).0;
    let mut tokenizer = HtmlTokenizerReader::new(text.as_bytes());
    let mut state = State::None;

    // Follow MimeKit's implementation of the HTML encoding prescan: explicit
    // html/head starts are required, and a body tag ends the search.
    // https://html.spec.whatwg.org/multipage/parsing.html#prescan-a-byte-stream-to-determine-its-encoding
    while let Some(token) = tokenizer
        .read_next_token()
        .expect("Latin-1 converted to an in-memory UTF-8 source")
    {
        let HtmlToken::Tag(tag) = token else {
            continue;
        };
        match tag.id() {
            HtmlTagId::Html if state == State::None => {
                if tag.is_end_tag() || tag.is_empty_element() {
                    return None;
                }
                state = State::Html;
            }
            HtmlTagId::Head if state == State::Html => {
                if tag.is_end_tag() || tag.is_empty_element() {
                    return None;
                }
                state = State::Head;
            }
            HtmlTagId::Body => return None,
            HtmlTagId::Meta if state == State::Head && !tag.is_end_tag() => {
                let mut seen_equiv = false;
                let mut seen_content = false;
                let mut seen_charset = false;
                let mut need_pragma = None;
                let mut got_pragma = false;
                let mut charset = None;
                for attribute in tag.attributes() {
                    let Some(value) = attribute.value() else {
                        continue;
                    };
                    // The first valued occurrence of each attribute wins;
                    // valueless duplicates do not reserve the attribute.
                    match attribute.id() {
                        HtmlAttributeId::HttpEquiv if !seen_equiv => {
                            seen_equiv = true;
                            got_pragma = value.eq_ignore_ascii_case("Content-Type");
                        }
                        HtmlAttributeId::Content if !seen_content => {
                            seen_content = true;
                            if charset.is_none()
                                && let Ok(content_type) = ContentType::parse(value.as_bytes())
                                && let Some(label) = content_type.charset()
                                && !label.is_empty()
                            {
                                charset = Some(label.trim().to_owned());
                                need_pragma = Some(true);
                            }
                        }
                        HtmlAttributeId::Charset if !seen_charset => {
                            seen_charset = true;
                            if !value.is_empty() {
                                charset = Some(value.trim().to_owned());
                                need_pragma = Some(false);
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(need_pragma) = need_pragma
                    && (!need_pragma || got_pragma)
                    && let Some(label) = charset
                {
                    let label = if label.eq_ignore_ascii_case("x-user-defined") {
                        "windows-1252"
                    } else {
                        &label
                    };
                    if let Ok(mut charset) = Charset::from_label(label) {
                        let strict_utf8 = matches!(charset.name(), "utf-16" | "utf-16be");
                        if strict_utf8 {
                            charset = Charset::UTF8;
                        }
                        return Some(DetectedEncoding {
                            charset,
                            confidence: TextEncodingConfidence::Tentative,
                            strict_utf8,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    None
}

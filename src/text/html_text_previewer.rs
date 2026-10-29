// Adapted from MimeKit's Text/HtmlTextPreviewer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::HashMap,
    io::{self, BufRead},
};

use super::{
    HtmlContentMode, HtmlTagId, HtmlToken, HtmlTokenizerOptions, HtmlTokenizerReader,
    InvalidPreviewLength,
    plain_text_previewer::{Preview, validate_preview_length},
};

/// Produces short plain-text previews of explicit HTML body content.
///
/// Matches MimeKit's preview heuristics, not [`super::HtmlToText`] formatting:
/// fragments without a non-self-closing `body` tag produce an empty preview.
/// Paragraphs and breaks introduce spaces, image alternative text is included,
/// and ordered list items are numbered starting at one. List `start`/`value`
/// attributes are not interpreted. Text in a list/table container is suppressed
/// until an inner element allows it. CSS and `hidden` attributes are not evaluated.
///
/// Unicode whitespace and U+200B through U+200D collapse to ASCII spaces.
/// Leading/trailing spaces are removed. A further character in eligible text
/// after the limit replaces the last retained scalar with U+2026. Tags alone
/// do not cause truncation, and a trailing space is trimmed before replacement.
///
/// The default limit is 230; valid limits are 1..=1024 Unicode scalar values,
/// not UTF-8 bytes, grapheme clusters or MimeKit's UTF-16 code units.
/// Supplementary characters remain intact, but combining sequences may split.
///
/// Only preview output is bounded by this limit. A complete token, suppressed
/// content, or a document without a body can require arbitrarily much input.
/// The tokenizer retains its default structural depth limit, while this
/// previewer's independent open-element state grows with unclosed tags.
/// This is not a sanitizer, DOM renderer, charset decoder or MIME-body selector.
///
/// ```
/// use mimecraft::text::HtmlTextPreviewer;
///
/// let previewer = HtmlTextPreviewer::with_maximum_preview_length(12)?;
/// assert_eq!(previewer.preview("<body><p>Hello &amp; goodbye.</p></body>"),
///     "Hello & goo\u{2026}");
/// assert_eq!(previewer.preview("<p>No body tag</p>"), "");
/// # Ok::<(), mimecraft::text::InvalidPreviewLength>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HtmlTextPreviewer {
    maximum_preview_length: usize,
}

impl Default for HtmlTextPreviewer {
    fn default() -> Self {
        Self {
            maximum_preview_length: 230,
        }
    }
}

impl HtmlTextPreviewer {
    /// Creates a previewer with a maximum length of 230 Unicode scalar values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a previewer with an explicit output limit.
    ///
    /// # Errors
    /// Returns [`InvalidPreviewLength`] for a limit outside 1..=1024.
    pub fn with_maximum_preview_length(
        maximum_preview_length: usize,
    ) -> Result<Self, InvalidPreviewLength> {
        validate_preview_length(maximum_preview_length)?;
        Ok(Self {
            maximum_preview_length,
        })
    }

    /// Returns the maximum preview length in Unicode scalar values.
    pub fn maximum_preview_length(&self) -> usize {
        self.maximum_preview_length
    }

    /// Generates a preview from decoded HTML without modifying the previewer.
    ///
    /// Tokenization stops once eligible text requires truncation. EOF-truncated
    /// tags are discarded. Character references are decoded; script tokens,
    /// comments, declarations and CDATA are omitted.
    pub fn preview(&self, html: &str) -> String {
        self.preview_reader(&mut html.as_bytes())
            .expect("a UTF-8 string source cannot fail")
    }

    /// Generates a preview from the current position of a strict UTF-8 reader.
    ///
    /// Matches [`Self::preview`]. No BOM or charset detection is performed.
    /// The reader is borrowed, not closed or sought, but tokenization may
    /// consume ahead in chunks of up to 4096 bytes. Bytes already copied into
    /// the tokenizer are not returned when previewing stops: unlike
    /// [`super::PlainTextPreviewer::preview_reader`], this does not preserve the
    /// exact suffix following the preview. It does not drain the source.
    ///
    /// # Errors
    /// Propagates read failures and invalid UTF-8 encountered while completing
    /// tokens needed for the preview. Even text beyond the preview limit may
    /// be validated if it belongs to the same token. Interrupted reads retry.
    /// No partial preview is returned and consumed input is not restored.
    pub fn preview_reader<R: BufRead + ?Sized>(&self, reader: &mut R) -> io::Result<String> {
        let options = HtmlTokenizerOptions {
            ignore_truncated_tags: true,
            ..Default::default()
        };
        let mut tokenizer = HtmlTokenizerReader::with_options(reader, options)
            .expect("positive default tokenizer depth");
        let mut preview = Preview::new(self.maximum_preview_length);
        let mut stack = OpenElements::default();
        let mut prefix = "";
        let mut body = false;
        let mut full = false;

        while !full {
            let Some(token) = tokenizer.read_next_token()? else {
                break;
            };
            match token {
                HtmlToken::Tag(tag) => {
                    if !tag.is_end_tag() {
                        if body {
                            match tag.id() {
                                HtmlTagId::Image => {
                                    if let Some(alt) = tag.attributes().get("alt") {
                                        full = preview.append(prefix, self.maximum_preview_length)
                                            || preview.append(
                                                alt.value().unwrap_or(""),
                                                self.maximum_preview_length,
                                            );
                                        prefix = "";
                                    }
                                }
                                HtmlTagId::Li => {
                                    if let Some(context) = stack.list_item_context() {
                                        if context.id == HtmlTagId::Ol {
                                            context.list_index += 1;
                                            full = preview.append(
                                                &format!(" {}. ", context.list_index),
                                                self.maximum_preview_length,
                                            );
                                            prefix = "";
                                        } else {
                                            // Unordered items use spacing, not a bullet marker.
                                            prefix = " ";
                                        }
                                    }
                                }
                                HtmlTagId::Br | HtmlTagId::P => prefix = " ",
                                _ => {}
                            }
                            let mode = tokenizer
                                .tokenizer()
                                .tree_construction()
                                .last_tag()
                                .expect("tag has structural result")
                                .content_mode;
                            if !tag.is_empty_element() || mode != HtmlContentMode::Data {
                                stack.push(tag.id());
                            }
                        } else if tag.id() == HtmlTagId::Body && !tag.is_empty_element() {
                            body = true;
                        }
                    } else if tag.id() == HtmlTagId::Body {
                        stack.clear();
                        body = false;
                    } else {
                        stack.pop(tag.id());
                    }
                }
                HtmlToken::Data(data) if body && !stack.suppresses_content() => {
                    full = preview.append(prefix, self.maximum_preview_length)
                        || preview.append(data.data(), self.maximum_preview_length);
                    prefix = "";
                }
                _ => {}
            }
        }
        Ok(preview.finish(full))
    }
}

struct Element {
    id: HtmlTagId,
    list_index: u64,
    suppress_inner_content: bool,
    // Links in the document-order list of open elements.
    outer: Option<usize>,
    inner: Option<usize>,
    // The next outer open element with the same tag identifier.
    outer_same_id: Option<usize>,
    // Increasing push order determines which of two list elements is innermost.
    sequence: u64,
}

/// Open elements form a doubly-linked list so a matching end can remove an
/// element from the middle without closing its children. Identifier chains
/// find that element in constant time; sequence numbers select the innermost
/// list without a depth scan. Otherwise many unmatched ends or list items
/// following unclosed tags would take quadratic time.
#[derive(Default)]
struct OpenElements {
    elements: Vec<Option<Element>>,
    free: Vec<usize>,
    innermost_by_id: HashMap<HtmlTagId, usize>,
    innermost: Option<usize>,
    sequence: u64,
}

impl OpenElements {
    fn element(&self, index: usize) -> &Element {
        self.elements[index].as_ref().expect("live element")
    }

    fn element_mut(&mut self, index: usize) -> &mut Element {
        self.elements[index].as_mut().expect("live element")
    }

    fn suppresses_content(&self) -> bool {
        // This is the innermost element's own flag, not inherited suppression.
        // For example, a table suppresses text but its cells allow text again.
        self.innermost
            .is_some_and(|index| self.element(index).suppress_inner_content)
    }

    fn push(&mut self, id: HtmlTagId) {
        let index = self.free.pop().unwrap_or_else(|| {
            self.elements.push(None);
            self.elements.len() - 1
        });
        let outer_same_id = self.innermost_by_id.insert(id, index);
        let element = Element {
            id,
            list_index: 0,
            suppress_inner_content: matches!(
                id,
                HtmlTagId::Ol
                    | HtmlTagId::Script
                    | HtmlTagId::Style
                    | HtmlTagId::Table
                    | HtmlTagId::TBody
                    | HtmlTagId::THead
                    | HtmlTagId::Tr
                    | HtmlTagId::Ul
            ),
            outer: self.innermost,
            inner: None,
            outer_same_id,
            sequence: self.sequence,
        };
        self.sequence += 1;
        if let Some(outer) = self.innermost {
            self.element_mut(outer).inner = Some(index);
        }
        self.elements[index] = Some(element);
        self.innermost = Some(index);
    }

    fn pop(&mut self, id: HtmlTagId) {
        let Some(index) = self.innermost_by_id.remove(&id) else {
            return;
        };
        let element = self.elements[index].take().expect("live element");
        if let Some(outer) = element.outer_same_id {
            self.innermost_by_id.insert(id, outer);
        }
        if let Some(inner) = element.inner {
            self.element_mut(inner).outer = element.outer;
        } else {
            self.innermost = element.outer;
        }
        if let Some(outer) = element.outer {
            self.element_mut(outer).inner = element.inner;
        }
        self.free.push(index);
    }

    fn list_item_context(&mut self) -> Option<&mut Element> {
        let ordered = self.innermost_by_id.get(&HtmlTagId::Ol).copied();
        let unordered = self.innermost_by_id.get(&HtmlTagId::Ul).copied();
        let index = match (ordered, unordered) {
            (Some(ol), Some(ul)) => {
                if self.element(ol).sequence > self.element(ul).sequence {
                    ol
                } else {
                    ul
                }
            }
            (Some(index), None) | (None, Some(index)) => index,
            (None, None) => return None,
        };
        Some(self.element_mut(index))
    }

    fn clear(&mut self) {
        self.innermost_by_id.clear();
        self.elements.clear();
        self.free.clear();
        self.innermost = None;
        self.sequence = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_removal_and_slot_reuse_preserve_chains() {
        let mut stack = OpenElements::default();
        stack.push(HtmlTagId::Ol);
        stack.push(HtmlTagId::B);
        stack.push(HtmlTagId::Ul);
        assert_eq!(stack.list_item_context().unwrap().id, HtmlTagId::Ul);
        stack.pop(HtmlTagId::Ol);
        assert_eq!(stack.list_item_context().unwrap().id, HtmlTagId::Ul);
        stack.push(HtmlTagId::Ol);
        assert_eq!(stack.elements.len(), 3);
        assert_eq!(stack.list_item_context().unwrap().id, HtmlTagId::Ol);
        stack.pop(HtmlTagId::Ul);
        stack.pop(HtmlTagId::Ol);
        assert!(stack.list_item_context().is_none());
        assert!(!stack.suppresses_content());
        for _ in 0..100_000 {
            stack.push(HtmlTagId::Ol);
            stack.push(HtmlTagId::Ol);
            stack.pop(HtmlTagId::Ol);
            assert_eq!(stack.list_item_context().unwrap().list_index, 0);
            stack.pop(HtmlTagId::Ol);
        }
        assert_eq!(stack.elements.len(), 3);
        stack.clear();
        assert!(stack.innermost.is_none());
        assert!(stack.elements.is_empty());
        stack.push(HtmlTagId::Ul);
        assert!(stack.suppresses_content());
    }
}

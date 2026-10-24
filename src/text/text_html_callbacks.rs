// Adapted from MimeKit's Text/TextToHtml.cs and Text/FlowedToHtml.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    HtmlAttribute, HtmlAttributeId, HtmlTagContext, HtmlTagId, HtmlWriter, next_line, unquote,
    url_scanner,
};

pub(super) struct TextHtmlCallbacks<F> {
    writer: HtmlWriter,
    callback: F,
    quotes: Vec<HtmlTagContext>,
    suppressed: bool,
}

impl<F: FnMut(&mut HtmlTagContext, &mut HtmlWriter)> TextHtmlCallbacks<F> {
    pub fn new(prefix: String, callback: F) -> Self {
        let mut writer = HtmlWriter::new();
        writer.write_markup_text(&prefix);
        Self {
            writer,
            callback,
            quotes: Vec::new(),
            suppressed: false,
        }
    }

    fn open(&mut self, id: HtmlTagId, attribute: Option<HtmlAttribute>) -> HtmlTagContext {
        let mut context = HtmlTagContext::new(id, attribute.into_iter().collect());
        (self.callback)(&mut context, &mut self.writer);
        context
    }

    fn close(&mut self, context: &mut HtmlTagContext) {
        if context.delete_end_tag {
            return;
        }
        context.mark_end_tag();
        if context.invoke_callback_for_end_tag {
            (self.callback)(context, &mut self.writer);
        } else {
            context
                .write_tag(&mut self.writer, false)
                .expect("generated end tag");
        }
    }

    fn quote_depth(&mut self, depth: usize) {
        if self.suppressed {
            // Descendants of the first suppressing quote have no callbacks or
            // output. Absolute input depths let us skip storing their contexts.
            if depth >= self.quotes.len() {
                return;
            }
            // Clear suppression before closing its owner so that the owner's
            // own end tag/callback survives, unlike its descendants.
            self.suppressed = false;
        }
        while self.quotes.len() > depth {
            let mut context = self.quotes.pop().expect("open quote");
            self.close(&mut context);
        }
        while self.quotes.len() < depth {
            let context = self.open(HtmlTagId::BlockQuote, None);
            self.suppressed = context.suppress_inner_content;
            self.quotes.push(context);
            if self.suppressed {
                break;
            }
        }
    }

    fn write_text(&mut self, mut text: &str) {
        while let Some(found) = url_scanner::scan(text) {
            // Write the unmatched prefix before handing the link to the callback.
            // The scanner still sees the joined paragraph, not individual lines.
            if found.start > 0 {
                self.writer.write_text(&text[..found.start]);
            }
            let label = &text[found.start..found.end];
            let href = format!("{}{label}", found.prefix);
            let attribute =
                HtmlAttribute::from_id(HtmlAttributeId::Href, Some(href)).expect("href attribute");
            let mut context = self.open(HtmlTagId::A, Some(attribute));
            if !context.suppress_inner_content {
                self.writer.write_text(label);
            }
            self.close(&mut context);
            text = &text[found.end..];
            if text.is_empty() {
                return;
            }
        }
        self.writer.write_text(text);
    }

    pub fn write_plain_text(&mut self, mut text: &str) {
        while let Some(line) = next_line(&mut text) {
            let (line, depth) = unquote(line);
            self.quote_depth(depth);
            if !self.suppressed {
                self.write_text(line);
                self.open(HtmlTagId::Br, None);
            }
        }
    }

    pub fn write_paragraph(&mut self, paragraph: &str, depth: usize, newline: &str) {
        self.quote_depth(depth);
        if self.suppressed {
            return;
        }
        let id = if paragraph.is_empty() {
            HtmlTagId::Br
        } else {
            HtmlTagId::P
        };
        let mut context = self.open(id, None);
        if !paragraph.is_empty() {
            if !context.suppress_inner_content {
                self.write_text(paragraph);
            }
            self.close(&mut context);
        }
        // Flowed conversion uses DeleteTag, not DeleteEndTag, to decide whether
        // to emit the paragraph newline, including after the end-tag callback.
        if !context.delete_tag {
            self.writer.write_markup_text(newline);
        }
    }

    pub fn finish(mut self) -> String {
        // EOF must honor the same deletion and suppression decisions as a
        // quote-depth change; writing raw closing blockquotes would bypass them.
        self.quote_depth(0);
        self.writer.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppressed_quote_descendants_need_no_context_storage() {
        let mut callback_count = 0;
        let mut renderer = TextHtmlCallbacks::new(
            String::new(),
            |context: &mut HtmlTagContext, _: &mut HtmlWriter| {
                callback_count += 1;
                context.delete_end_tag = true;
                context.suppress_inner_content = true;
            },
        );
        renderer.quote_depth(1_000_000);
        assert_eq!(renderer.quotes.len(), 1);
        for depth in [500_000, 1, 700_000] {
            renderer.quote_depth(depth);
            assert_eq!(renderer.quotes.len(), 1);
        }
        renderer.quote_depth(0);
        assert!(renderer.quotes.is_empty());
        assert!(!renderer.suppressed);
        assert_eq!(renderer.finish(), "");
        assert_eq!(callback_count, 1);
    }
}

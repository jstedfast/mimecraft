// Adapted from MimeKit's Text/HtmlToHtml.cs, HtmlNoScriptHandling.cs,
// and HtmlTagContextStack.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
    num::NonZeroUsize,
};

use super::{
    HeaderFooterFormat, HtmlContentMode, HtmlTagContext, HtmlTagDisposition, HtmlTagId,
    HtmlTagToken, HtmlToken, HtmlTokenizerOptions, HtmlTokenizerReader, HtmlTreeConstruction,
    HtmlWriter, text_to_html::write_decoration,
};

/// How HTML rewriting tokenizes and emits `noscript` elements.
///
/// Modes retaining the wrapper must match the eventual renderer's scripting
/// setting. Otherwise markup can bypass tag callbacks: raw text can become
/// markup, or a hidden `</noscript>` in a comment/raw element can end the
/// renderer's wrapper. None of these modes makes the converter a sanitizer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HtmlNoScriptHandling {
    /// Parse fallback markup and omit the wrapper, independent of renderer
    /// scripting settings. Fallback content is consequently always displayed.
    #[default]
    Unwrap,
    /// Preserve the wrapper and tokenize its contents as raw text.
    ScriptingEnabled,
    /// Preserve the wrapper and tokenize its contents as normal markup.
    ScriptingDisabled,
}

/// Rewrites decoded HTML using MimeKit's token and callback rules.
///
/// Preserves character references in text, re-encodes attribute values, writes
/// recognized void tags with `/>`, and drops EOF-truncated tags. By default
/// retains comments and document structure and unwraps `noscript`.
/// Does not synthesize document wrappers or missing end tags at EOF.
///
/// Uses the sans-I/O tokenizer with strict UTF-8 reader/writer adapters.
/// Malformed HTML is recovered, not rejected. There is no charset/BOM detection,
/// DOM construction, CSS/URL validation, or compliance logging here. This is
/// **not an HTML sanitizer**; callbacks and decorations can emit trusted markup.
///
/// Only tokenizer structural depth is limited. Token sizes, output length and
/// callback-context counts are not capped; repeated elements can share a
/// tokenizer entry while requiring separate callback flags.
///
/// ```
/// use mimecraft::text::HtmlToHtml;
///
/// let converter = HtmlToHtml { output_html_fragment: true, ..HtmlToHtml::new() };
/// assert_eq!(converter.convert("<html><head><title>T</title></head>\
///     <body><noscript><img alt='A &amp; B'></noscript></body></html>")?,
///     "<img alt=\"A &amp; B\"/>");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlToHtml {
    /// Drops comments before consulting any tag callback; defaults to false.
    pub filter_comments: bool,
    /// Omits doctype/html/body wrappers and head content without callbacks.
    /// Head suppression ends at the first token that implicitly starts a body.
    pub output_html_fragment: bool,
    /// How `noscript` fallback is parsed and written; defaults to unwrapping.
    pub no_script_handling: HtmlNoScriptHandling,
    /// Maximum distinct tokenizer stack entries; defaults to 4096.
    /// Identical nested runs share entries. The overflowing start tag is the
    /// last callback-visible tag; subsequent input is encoded literal text.
    pub max_element_depth: NonZeroUsize,
    /// Trusted decoration before the converted document.
    pub header: Option<String>,
    /// Header interpretation; defaults to plain text with `TextToHtml` linking.
    pub header_format: HeaderFooterFormat,
    /// Trusted decoration after successful body conversion.
    pub footer: Option<String>,
    /// Footer interpretation; defaults to plain text with `TextToHtml` linking.
    pub footer_format: HeaderFooterFormat,
}

impl Default for HtmlToHtml {
    fn default() -> Self {
        Self {
            filter_comments: false,
            output_html_fragment: false,
            no_script_handling: HtmlNoScriptHandling::Unwrap,
            max_element_depth: NonZeroUsize::new(4096).unwrap(),
            header: None,
            header_format: HeaderFooterFormat::Text,
            footer: None,
            footer_format: HeaderFooterFormat::Text,
        }
    }
}

impl HtmlToHtml {
    /// Creates a converter with default options and no decorations.
    pub fn new() -> Self {
        Self::default()
    }

    /// Converts a decoded HTML string. Each call starts fresh parser state.
    ///
    /// # Errors
    /// Uses the same fallible interface as [`Self::convert_reader`]; a valid
    /// string source and in-memory destination have no external I/O failures.
    pub fn convert(&self, html: &str) -> io::Result<String> {
        self.convert_reader(html.as_bytes())
    }

    /// Converts a strict UTF-8 buffered source to an owned string.
    ///
    /// # Errors
    /// Propagates read failures and invalid UTF-8, without a partial result.
    pub fn convert_reader<R: BufRead>(&self, reader: R) -> io::Result<String> {
        self.convert_reader_with_tag_callback(reader, default_callback)
    }

    /// Converts a string with a callback deciding which tags to write.
    ///
    /// Call [`HtmlTagContext::write_tag`] to keep a tag, write replacement
    /// markup, or write nothing to omit it. The deletion flag is bookkeeping,
    /// not retroactive removal. Suppression hides descendants and their
    /// callbacks. Requested end callbacks take precedence over end deletion.
    ///
    /// Original case, unknown names and attributes are available. End callbacks
    /// receive end-token attributes with the start callback's flags copied.
    /// Implicit foreign closures can generate end callbacks. A callback can
    /// also see unmatched ends, but not unwrapped `noscript`, removed document
    /// structure, or tags inside suppressed content.
    ///
    /// The callback writer is a periodically drained staging buffer, not the
    /// accumulated document. Drains preserve pending tag/attribute state.
    /// Decorations bypass callbacks. Callback output is trusted and not parsed.
    ///
    /// # Errors
    /// Returns conversion errors. Callback writer operations return their own
    /// errors to the callback, as with the other HTML-generating converters.
    pub fn convert_with_tag_callback(
        &self,
        html: &str,
        callback: impl FnMut(&mut HtmlTagContext, &mut HtmlWriter),
    ) -> io::Result<String> {
        self.convert_reader_with_tag_callback(html.as_bytes(), callback)
    }

    /// Reader counterpart to [`Self::convert_with_tag_callback`].
    ///
    /// # Errors
    /// Propagates source/UTF-8 failures without a partial string.
    pub fn convert_reader_with_tag_callback<R: BufRead>(
        &self,
        reader: R,
        callback: impl FnMut(&mut HtmlTagContext, &mut HtmlWriter),
    ) -> io::Result<String> {
        let mut output = Vec::new();
        self.convert_to_with_tag_callback(reader, &mut output, callback)?;
        Ok(String::from_utf8(output).expect("HTML writer emits UTF-8"))
    }

    /// Converts a strict UTF-8 source to a UTF-8 destination incrementally.
    ///
    /// The destination is neither flushed nor closed. Pass borrowed streams to
    /// retain ownership. Interrupted reads/writes retry. On error, partial output
    /// and consumed/read-ahead input are not rolled back; conversion cannot
    /// resume, and the footer is not written.
    ///
    /// # Errors
    /// Propagates source, UTF-8 and destination failures.
    pub fn convert_to<R: BufRead, W: Write + ?Sized>(
        &self,
        reader: R,
        writer: &mut W,
    ) -> io::Result<()> {
        self.convert_to_with_tag_callback(reader, writer, default_callback)
    }

    /// Writer counterpart to [`Self::convert_with_tag_callback`].
    ///
    /// # Errors
    /// Propagates the same failures as [`Self::convert_to`].
    pub fn convert_to_with_tag_callback<R: BufRead, W: Write + ?Sized>(
        &self,
        reader: R,
        writer: &mut W,
        callback: impl FnMut(&mut HtmlTagContext, &mut HtmlWriter),
    ) -> io::Result<()> {
        write_header_footer(writer, self.header.as_deref(), self.header_format)?;
        let options = HtmlTokenizerOptions {
            decode_character_references: false,
            scripting_enabled: self.no_script_handling == HtmlNoScriptHandling::ScriptingEnabled,
            ignore_truncated_tags: true,
            max_element_depth: self.max_element_depth.get(),
        };
        let mut tokenizer =
            HtmlTokenizerReader::with_options(reader, options).expect("nonzero element depth");
        tokenizer.tokenizer_mut().set_track_closed_elements(true);
        let mut converter = Converter {
            options: self,
            callback,
            writer: HtmlWriter::new(),
            stack: ContextStack::default(),
            head_open: false,
            head_done: false,
            head_template_depth: 0,
            raw_text: false,
        };
        while let Some(token) = tokenizer.read_next_token()? {
            converter.process(token, tokenizer.tokenizer().tree_construction());
            tokenizer.tokenizer_mut().clear_closed_elements();
            if converter.writer.as_str().len() >= 4096 {
                converter.writer.drain_to(writer)?;
            }
        }
        converter.writer.flush();
        converter.writer.drain_to(writer)?;
        write_header_footer(writer, self.footer.as_deref(), self.footer_format)
    }
}

fn default_callback(context: &mut HtmlTagContext, writer: &mut HtmlWriter) {
    context
        .write_tag(writer, true)
        .expect("tokenizer tag names are valid");
}

fn write_header_footer<W: Write + ?Sized>(
    writer: &mut W,
    value: Option<&str>,
    format: HeaderFooterFormat,
) -> io::Result<()> {
    let mut decoration = String::new();
    write_decoration(&mut decoration, value, format);
    writer.write_all(decoration.as_bytes())
}

#[derive(Clone, Copy)]
struct Flags {
    delete_tag: bool,
    delete_end: bool,
    invoke_end: bool,
    suppress: bool,
}

impl Flags {
    fn from_context(context: &HtmlTagContext) -> Self {
        Self {
            delete_tag: context.delete_tag,
            delete_end: context.delete_end_tag,
            invoke_end: context.invoke_callback_for_end_tag,
            suppress: context.suppress_inner_content,
        }
    }

    fn apply(self, context: &mut HtmlTagContext) {
        context.delete_tag = self.delete_tag;
        context.delete_end_tag = self.delete_end;
        context.invoke_callback_for_end_tag = self.invoke_end;
        context.suppress_inner_content = self.suppress;
    }
}

/// Case-insensitive name chains and a suppression count avoid O(depth) scans
/// for every token or unmatched end. Only the matching innermost element is
/// removed, not its children. Separate vectors replace linked context objects;
/// only flag snapshots are needed for future end callbacks.
#[derive(Default)]
struct ContextStack {
    open: HashMap<String, Vec<Flags>>,
    suppressed: usize,
}

impl ContextStack {
    fn push(&mut self, context: &HtmlTagContext) {
        // Snapshot suppression when opened, keeping the count consistent even
        // if a subsequent end callback changes its copied context.
        let flags = Flags::from_context(context);
        self.suppressed += usize::from(flags.suppress);
        self.open
            .entry(context.tag_name().to_ascii_lowercase())
            .or_default()
            .push(flags);
    }

    fn peek(&self, name: &str) -> Option<Flags> {
        self.open.get(&name.to_ascii_lowercase())?.last().copied()
    }

    fn pop(&mut self, name: &str) -> Option<Flags> {
        let name = name.to_ascii_lowercase();
        let chain = self.open.get_mut(&name)?;
        let flags = chain.pop()?;
        self.suppressed -= usize::from(flags.suppress);
        if chain.is_empty() {
            self.open.remove(&name);
        }
        Some(flags)
    }

    fn suppresses_content(&self) -> bool {
        self.suppressed != 0
    }
}

struct Converter<'a, F> {
    options: &'a HtmlToHtml,
    callback: F,
    writer: HtmlWriter,
    stack: ContextStack,
    head_open: bool,
    head_done: bool,
    head_template_depth: usize,
    raw_text: bool,
}

fn structure(id: HtmlTagId) -> bool {
    matches!(id, HtmlTagId::Html | HtmlTagId::Head | HtmlTagId::Body)
}

fn template(tag: &HtmlTagToken) -> bool {
    tag.id() == HtmlTagId::Unknown && tag.name().eq_ignore_ascii_case("template")
}

// Other start tags implicitly close head under the "in head" insertion rules
// and are inserted into the body instead.
fn allowed_in_head(tag: &HtmlTagToken) -> bool {
    use HtmlTagId::*;
    matches!(
        tag.id(),
        Html | Head
            | Base
            | BaseFont
            | BgSound
            | Link
            | Meta
            | Title
            | NoScript
            | NoFrames
            | Style
            | Script
    ) || template(tag)
}

impl<F: FnMut(&mut HtmlTagContext, &mut HtmlWriter)> Converter<'_, F> {
    // Fragments remove head content, but only until a token implicitly starts
    // the body. Removing later content could leave foreign elements open and
    // make the output parse differently from the input.
    fn end_head(&mut self) {
        if self.head_open {
            self.stack.pop("head");
            self.head_open = false;
        }
        self.head_done = true;
    }

    fn process_end(&mut self, tag: HtmlTagToken, implicit: bool, tree: &HtmlTreeConstruction) {
        let unwrap = self.options.no_script_handling == HtmlNoScriptHandling::Unwrap;
        // Unwrapped noscript starts were never written, including foreign
        // noscript elements that are now implicitly closed.
        if unwrap && tag.id() == HtmlTagId::NoScript {
            return;
        }
        let flags = if !implicit
            && tree
                .last_tag()
                .is_some_and(|tag| tag.disposition == HtmlTagDisposition::Ignored)
        {
            // An ignored end leaves its context open. Popping it now would
            // select the wrong context when the element is eventually closed.
            if unwrap && tree.is_noscript_open() {
                // Noscript may be the special element blocking this end.
                // Without its wrapper the end would close foreign content.
                return;
            }
            self.stack.peek(tag.name())
        } else {
            self.stack.pop(tag.name())
        };
        if self.stack.suppresses_content() {
            return;
        }
        if let Some(flags) = flags {
            if flags.invoke_end {
                let mut context = HtmlTagContext::from_token(tag);
                flags.apply(&mut context);
                (self.callback)(&mut context, &mut self.writer);
            } else if !flags.delete_end {
                self.writer
                    .write_end_tag(tag.name())
                    .expect("tokenizer tag names are valid");
            }
        } else if !(self.options.output_html_fragment && structure(tag.id())) {
            // A self-closing non-void start may have no callback context even
            // though browsers kept it open. Its explicit end is still needed
            // so ends for foreign elements beneath it will be honored.
            (self.callback)(&mut HtmlTagContext::from_token(tag), &mut self.writer);
        }
    }

    fn process(&mut self, token: HtmlToken, tree: &HtmlTreeConstruction) {
        let fragment = self.options.output_html_fragment;
        let HtmlToken::Tag(tag) = token else {
            match &token {
                HtmlToken::Comment(_) if self.options.filter_comments => return,
                HtmlToken::DocType(_) if fragment => return,
                HtmlToken::Data(data)
                    if fragment
                        && !self.head_done
                        && !self.raw_text
                        && self.head_template_depth == 0
                        && !data
                            .data()
                            .bytes()
                            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 12)) =>
                {
                    self.end_head()
                }
                _ => {}
            }
            if !self.stack.suppresses_content() {
                self.writer.write_token(&token);
            }
            return;
        };

        // Tags such as body/img or </div> can close SVG/MathML implicitly.
        // The callback might omit that tag, leaving output in foreign content.
        // Explicitly close everything through the outermost foreign element,
        // including HTML elements above it: a foreign end is honored only
        // when that element is current. This also preserves idempotence.
        let closed = tree.closed_elements();
        if let Some(last) = closed.iter().rposition(|element| element.is_foreign) {
            for element in &closed[..=last] {
                for _ in 0..element.count {
                    self.process_end(HtmlTagToken::end(element.name.clone()), true, tree);
                }
            }
        }

        // Frameset-ignored tags never switched the tokenizer's state. Drop
        // them so removing the frameset cannot make them take effect later.
        if tree.is_in_frameset()
            && !matches!(
                tag.id(),
                HtmlTagId::FrameSet | HtmlTagId::Frame | HtmlTagId::NoFrames
            )
        {
            return;
        }
        // An ignored frameset start must not become effective after stripping
        // the body/head tokens that cleared the input's frameset-ok flag.
        if tag.id() == HtmlTagId::FrameSet && !tag.is_end_tag() && !tree.is_in_frameset() {
            return;
        }
        // Fallback was tokenized as markup. Removing its wrapper makes the
        // result independent of the renderer's scripting setting.
        if self.options.no_script_handling == HtmlNoScriptHandling::Unwrap
            && tag.id() == HtmlTagId::NoScript
        {
            return;
        }
        if !tag.is_end_tag() {
            if fragment && !self.head_done {
                if !self.head_open {
                    // Body content makes subsequent head starts ineffective.
                    if !allowed_in_head(&tag) {
                        self.head_done = true;
                    }
                } else if template(&tag) {
                    self.head_template_depth += 1;
                } else if self.head_template_depth == 0 && !allowed_in_head(&tag) {
                    self.end_head();
                }
            }
            let mode = tree
                .last_tag()
                .expect("tag has structural result")
                .content_mode;
            if fragment && structure(tag.id()) {
                // Structure bypasses the callback. Browsers ignore head's
                // self-closing flag and ignore subsequent head starts.
                if tag.id() == HtmlTagId::Head {
                    if !self.head_done && !self.head_open {
                        let mut context = HtmlTagContext::from_token(tag);
                        context.suppress_inner_content = true;
                        context.delete_end_tag = true;
                        context.delete_tag = true;
                        self.stack.push(&context);
                        self.head_open = true;
                    }
                } else if !tag.is_empty_element() {
                    let mut context = HtmlTagContext::from_token(tag);
                    context.delete_end_tag = true;
                    context.delete_tag = true;
                    self.stack.push(&context);
                }
            } else {
                // Self-closing script/style still enter raw text: retain a
                // context so suppression applies to the following content.
                let push = !tag.is_empty_element() || mode != HtmlContentMode::Data;
                let mut context = HtmlTagContext::from_token(tag);
                if !self.stack.suppresses_content() {
                    (self.callback)(&mut context, &mut self.writer);
                }
                if push {
                    self.stack.push(&context);
                }
            }
            self.raw_text = mode != HtmlContentMode::Data;
        } else {
            self.raw_text = false;
            if fragment && self.head_open {
                if self.head_template_depth > 0 {
                    if template(&tag) {
                        self.head_template_depth -= 1;
                    } else if tag.id() == HtmlTagId::Head {
                        return;
                    }
                } else if tag.id() == HtmlTagId::Head {
                    self.process_end(tag, false, tree);
                    self.head_open = false;
                    self.head_done = true;
                    return;
                } else if matches!(tag.id(), HtmlTagId::Body | HtmlTagId::Html | HtmlTagId::Br) {
                    self.end_head();
                }
            }
            self.process_end(tag, false, tree);
        }
    }
}

// Adapted from MimeKit's Text/HtmlTreeConstruction.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use super::{
    HtmlAttributeId, HtmlNamespace, HtmlTagId, HtmlTagToken,
    html_open_element_stack::{
        BUTTON_SCOPE, Entry, HEADING, HTML_INTEGRATION, LIST_ITEM_SCOPE, LIST_ITEM_STOP,
        MATH_INTEGRATION, OpenElementStack, Predicate, SCOPE, SPECIAL, TABLE_SCOPE,
    },
};

/// The lexical content mode selected by HTML structural processing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HtmlContentMode {
    /// Ordinary markup and character references.
    #[default]
    Data,
    /// Character references and a matching end tag, as in title/textarea.
    RcData,
    /// Raw text and a matching end tag, as in style.
    RawText,
    /// Script text, including its escaped and double-escaped states.
    ScriptData,
    /// Literal text through EOF; also selected after structural depth overflow.
    PlainText,
}

/// How the tree builder treated a tag, independently of its serialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlTagDisposition {
    /// Neither ignored nor merged; does not necessarily insert an element.
    Normal,
    /// The tag did not take effect under the current structural rules.
    Ignored,
    /// Attributes were offered to an already-created html/body element.
    AttributesMerged,
}

/// Structural processing results for one tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HtmlTagResult {
    /// The lexical mode for content following the tag.
    pub content_mode: HtmlContentMode,
    /// Whether the tag was ignored, merged, or processed normally.
    pub disposition: HtmlTagDisposition,
    /// Whether the tag broke out of SVG or MathML content.
    pub broke_out_of_foreign_content: bool,
}

/// An element, or folded run of elements, closed implicitly by a tag or text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlClosedElement {
    /// Original name of the first element in the run.
    pub name: String,
    /// Number of consecutive identical elements closed.
    pub count: usize,
    /// Whether the elements belonged to SVG or MathML rather than HTML.
    pub is_foreign: bool,
}

/// A zero maximum element depth was requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidHtmlElementDepth;

impl fmt::Display for InvalidHtmlElementDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("maximum HTML element depth must be positive")
    }
}

impl Error for InvalidHtmlElementDepth {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InsertionMode {
    BeforeHead,
    InHead,
    InHeadNoScript,
    AfterHead,
}

/// Structural state for consumers of HTML tokens; not a DOM or an HTML parser.
///
/// Follows MimeKit's partial tree-construction model to select lexical modes
/// and distinguish HTML from SVG/MathML. For example, HTML `style` starts raw
/// text, but SVG `style` does not, and CDATA is recognized only in foreign
/// content. Ignoring this context can hide markup from filtering callbacks.
///
/// Tracks implied closures, scope boundaries, table structure, form pointers,
/// head/noscript and frameset rules, and adoption-agency stack updates. Like
/// MimeKit, it does not implement a DOM, active-formatting reconstruction,
/// foster parenting, or select/template insertion modes. It is **not a
/// sanitizer**, and supplying arbitrary tokens does not validate their origin.
///
/// Feed complete tags with [`Self::process_tag`], and decoded ordinary character
/// data or CDATA with [`Self::process_character_data`]. Do not feed script,
/// RAWTEXT, or RCDATA contents as ordinary character data. Consumers must honor
/// the returned content mode when identifying subsequent tokens.
///
/// Consecutive identical elements share one stack entry. Exceeding the maximum
/// distinct-entry depth permanently selects [`HtmlContentMode::PlainText`];
/// consumers must treat the remainder as literal text, not trust later markup.
/// This limit differs from the MIME parser's root-inclusive hard-error budget.
///
/// ```
/// use mimecraft::text::{HtmlContentMode, HtmlTagToken, HtmlTreeConstruction};
///
/// let mut tree = HtmlTreeConstruction::new();
/// tree.process_tag(&HtmlTagToken::start("svg", vec![]));
/// assert!(tree.is_in_foreign_content());
/// let result = tree.process_tag(&HtmlTagToken::start("style", vec![]));
/// assert_eq!(result.content_mode, HtmlContentMode::Data);
/// ```
#[derive(Debug)]
pub struct HtmlTreeConstruction {
    stack: OpenElementStack,
    mode: InsertionMode,
    // AfterHead covers both "after head" and "in body"; body_created distinguishes them.
    body_created: bool,
    html_created: bool,
    frameset_ok: bool,
    in_frameset: bool,
    frameset_depth: usize,
    // A form pointer can remain set after its stack entry has been popped.
    form_pointer_set: bool,
    form_pointer_index: Option<usize>,
    popped_through: bool,
    scripting_enabled: bool,
    namespace: HtmlNamespace,
    last_tag: Option<HtmlTagResult>,
    disposition: HtmlTagDisposition,
    broke_out: bool,
}

impl Default for HtmlTreeConstruction {
    fn default() -> Self {
        Self {
            stack: OpenElementStack::new(4096),
            mode: InsertionMode::BeforeHead,
            body_created: false,
            html_created: false,
            frameset_ok: true,
            in_frameset: false,
            frameset_depth: 0,
            form_pointer_set: false,
            form_pointer_index: None,
            popped_through: false,
            scripting_enabled: true,
            namespace: HtmlNamespace::Html,
            last_tag: None,
            disposition: HtmlTagDisposition::Normal,
            broke_out: false,
        }
    }
}

impl HtmlTreeConstruction {
    /// Creates structural state with a 4096-entry limit and scripting enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates state with a positive distinct-entry limit.
    ///
    /// # Errors
    /// Returns [`InvalidHtmlElementDepth`] for zero.
    pub fn with_max_element_depth(maximum: usize) -> Result<Self, InvalidHtmlElementDepth> {
        let mut tree = Self::new();
        tree.set_max_element_depth(maximum)?;
        Ok(tree)
    }

    /// Returns the maximum number of distinct stack entries.
    pub fn max_element_depth(&self) -> usize {
        self.stack.max_depth
    }

    /// Changes the limit without truncating existing state or clearing overflow.
    ///
    /// # Errors
    /// Zero returns [`InvalidHtmlElementDepth`] without changing the limit.
    pub fn set_max_element_depth(&mut self, maximum: usize) -> Result<(), InvalidHtmlElementDepth> {
        if maximum == 0 {
            return Err(InvalidHtmlElementDepth);
        }
        self.stack.max_depth = maximum;
        Ok(())
    }

    /// Returns whether structural tracking has permanently exceeded its limit.
    pub fn depth_exceeded(&self) -> bool {
        self.stack.depth_exceeded
    }

    /// Returns whether noscript content is processed with scripting enabled.
    pub fn scripting_enabled(&self) -> bool {
        self.scripting_enabled
    }

    /// Sets the policy for subsequent tags; does not reinterpret earlier input.
    pub fn set_scripting_enabled(&mut self, enabled: bool) {
        self.scripting_enabled = enabled;
    }

    /// Returns the namespace declared by html's xmlns attribute.
    ///
    /// This is not the adjusted current node's namespace. Unknown declarations
    /// select HTML for compatibility with MimeKit; SVG/MathML elements do not
    /// change this property.
    pub fn html_namespace(&self) -> HtmlNamespace {
        self.namespace
    }

    /// Returns whether the current node belongs to SVG or MathML.
    pub fn is_in_foreign_content(&self) -> bool {
        self.stack
            .entries
            .last()
            .is_some_and(|e| e.namespace != HtmlNamespace::Html)
    }

    /// Returns whether an HTML noscript element is open.
    pub fn is_noscript_open(&self) -> bool {
        self.stack.index_of(HtmlTagId::NoScript).is_some()
    }

    /// Returns whether the document entered a frameset insertion mode.
    pub fn is_in_frameset(&self) -> bool {
        self.in_frameset
    }

    /// Returns whether ordinary character data can still affect structural state.
    pub fn tracks_character_data(&self) -> bool {
        !self.in_frameset
            && (self.mode != InsertionMode::AfterHead || self.frameset_ok || !self.body_created)
    }

    /// Returns the last tag result, or `None` before any tag was processed.
    pub fn last_tag(&self) -> Option<HtmlTagResult> {
        self.last_tag
    }

    /// Returns whether implicit closures are being collected.
    pub fn tracks_closed_elements(&self) -> bool {
        self.stack.closed.is_some()
    }

    /// Enables closure collection, or disables it and discards the collection.
    ///
    /// Filtering out a foreign-content breakout tag can otherwise leave a
    /// browser in a different namespace. Consumers can write these explicit
    /// end tags instead. Enabling twice preserves the current collection.
    pub fn set_track_closed_elements(&mut self, enabled: bool) {
        if enabled {
            self.stack.closed.get_or_insert_with(Vec::new);
        } else {
            self.stack.closed = None;
        }
    }

    /// Borrows implicit closures in innermost-first order.
    ///
    /// Cleared on the next tag or explicit clear; character data may append.
    /// The element explicitly closed by an end tag is not included.
    pub fn closed_elements(&self) -> &[HtmlClosedElement] {
        self.stack.closed.as_deref().unwrap_or(&[])
    }

    /// Clears collected closures without changing the stack.
    pub fn clear_closed_elements(&mut self) {
        if let Some(closed) = &mut self.stack.closed {
            closed.clear();
        }
    }

    /// Processes one complete tag and selects the following lexical mode.
    ///
    /// This reports structural effects; it does not rewrite the supplied token.
    /// After depth overflow, further tags are ignored and return plaintext mode.
    pub fn process_tag(&mut self, tag: &HtmlTagToken) -> HtmlTagResult {
        self.clear_closed_elements();
        if self.depth_exceeded() {
            let result = HtmlTagResult {
                content_mode: HtmlContentMode::PlainText,
                disposition: HtmlTagDisposition::Ignored,
                broke_out_of_foreign_content: false,
            };
            self.last_tag = Some(result);
            return result;
        }
        self.disposition = HtmlTagDisposition::Normal;
        self.broke_out = false;
        let mut content_mode = if tag.is_end_tag() {
            self.process_end_tag(tag);
            HtmlContentMode::Data
        } else {
            self.process_start_tag(tag)
        };
        // Once structural tracking fails, the remainder must be literal text.
        if self.depth_exceeded() {
            content_mode = HtmlContentMode::PlainText;
        }
        let result = HtmlTagResult {
            content_mode,
            disposition: self.disposition,
            broke_out_of_foreign_content: self.broke_out,
        };
        self.last_tag = Some(result);
        result
    }

    /// Processes decoded ordinary data-state text or CDATA, skipping HTML spaces.
    ///
    /// Non-whitespace data inserts the body and closes a head noscript. NUL does
    /// not clear frameset-ok: browsers ignore it in HTML and replace it in
    /// foreign content without clearing that flag either. After depth overflow,
    /// character data no longer changes structural state.
    pub fn process_character_data(&mut self, text: &str) {
        if self.depth_exceeded() {
            return;
        }
        for character in text.chars() {
            if !self.tracks_character_data() {
                break;
            }
            if matches!(character, '\t' | '\n' | '\r' | '\x0c' | ' ') {
                continue;
            }
            self.html_created = true;
            // In-template character data is processed in-body without closing the head.
            if !self.is_template_open() {
                if self.mode == InsertionMode::InHeadNoScript {
                    self.pop_through(self.stack.index_of(HtmlTagId::NoScript).unwrap());
                }
                self.mode = InsertionMode::AfterHead;
                self.insert_implied_body();
            }
            if character != '\0' {
                self.frameset_ok = false;
            }
        }
    }

    fn is_template_open(&self) -> bool {
        self.stack
            .index_of_name(HtmlNamespace::Html, HtmlTagId::Unknown, "template")
            .is_some()
    }

    fn insert_implied_body(&mut self) {
        if !self.body_created {
            self.body_created = true;
            // The "anything else" after-head rules reset frameset-ok, even after a template.
            self.frameset_ok = true;
        }
    }

    fn pop_above(&mut self, index: Option<usize>) {
        self.stack.pop_above(index);
        if self
            .form_pointer_index
            .is_some_and(|i| i >= self.stack.entries.len())
        {
            self.form_pointer_index = None;
        }
    }

    fn pop_through(&mut self, index: usize) {
        self.popped_through = true;
        self.stack.pop_through(index);
        if self
            .form_pointer_index
            .is_some_and(|i| i >= self.stack.entries.len())
        {
            self.form_pointer_index = None;
        }
    }

    fn content_mode(&mut self, tag: &HtmlTagToken) -> HtmlContentMode {
        use HtmlTagId::*;
        // Self-closing spelling does not change non-void HTML content modes.
        match tag.id() {
            Style | Xmp | IFrame | NoEmbed | NoFrames => HtmlContentMode::RawText,
            Title | TextArea => HtmlContentMode::RcData,
            PlainText => HtmlContentMode::PlainText,
            Script => HtmlContentMode::ScriptData,
            NoScript if self.scripting_enabled => HtmlContentMode::RawText,
            Html if !tag.is_empty_element() => {
                if let Some(value) = tag
                    .attributes()
                    .iter()
                    .rev()
                    .find(|a| a.id() == HtmlAttributeId::XmlNs && a.value().is_some())
                    .and_then(|a| a.value())
                {
                    self.namespace = HtmlNamespace::from_url(value).unwrap_or(HtmlNamespace::Html);
                }
                HtmlContentMode::Data
            }
            _ => HtmlContentMode::Data,
        }
    }

    fn process_start_tag(&mut self, tag: &HtmlTagToken) -> HtmlContentMode {
        use HtmlTagId::*;
        let html_existed = self.html_created;
        let entry_mode = self.mode;
        self.html_created = true;
        // Only noframes changes lexical state in frameset insertion modes.
        if self.in_frameset {
            match tag.id() {
                Html => {
                    if !tag.attributes().is_empty() {
                        self.disposition = HtmlTagDisposition::AttributesMerged;
                    }
                }
                FrameSet if self.frameset_depth > 0 => {
                    self.frameset_depth = self.frameset_depth.saturating_add(1)
                }
                Frame if self.frameset_depth > 0 => {}
                NoFrames => {}
                _ => self.disposition = HtmlTagDisposition::Ignored,
            }
            return if tag.id() == NoFrames {
                HtmlContentMode::RawText
            } else {
                HtmlContentMode::Data
            };
        }
        if self.mode != InsertionMode::AfterHead && !self.process_head_start(tag) {
            self.disposition = HtmlTagDisposition::Ignored;
            return self.content_mode(tag);
        }
        if let Some(top) = self.stack.entries.last()
            && top.namespace != HtmlNamespace::Html
        {
            let html = if top.flags & HTML_INTEGRATION != 0 {
                true
            } else if top.flags & MATH_INTEGRATION != 0 {
                !name_is(tag, "mglyph") && !name_is(tag, "malignmark")
            } else {
                top.namespace == HtmlNamespace::MathMl
                    && top.name.eq_ignore_ascii_case("annotation-xml")
                    && name_is(tag, "svg")
            };
            if !html {
                if foreign_breakout(tag) {
                    // Pop to an HTML element or integration point, then reprocess as HTML.
                    self.pop_above(self.stack.nearest(Predicate::HtmlOrIntegrationPoint));
                    self.broke_out = true;
                } else {
                    let ns = top.namespace;
                    if !tag.is_empty_element() {
                        self.stack
                            .push(tag.name(), Unknown, ns, foreign_flags(ns, tag));
                    }
                    return HtmlContentMode::Data;
                }
            }
        }
        self.process_html_start(tag, entry_mode, html_existed);
        self.content_mode(tag)
    }

    fn process_head_start(&mut self, tag: &HtmlTagToken) -> bool {
        use HtmlTagId::*;
        // In-template tags do not close the head, even when normally handled in-body.
        if self.is_template_open() {
            return true;
        }
        match self.mode {
            InsertionMode::BeforeHead => {
                if tag.id() == Html {
                    return true;
                }
                // Any other start tag implies head and is reprocessed in-head.
                self.mode = InsertionMode::InHead;
            }
            InsertionMode::InHeadNoScript => {
                match tag.id() {
                    Html | BaseFont | BgSound | Link | Meta | NoFrames | Style => return true,
                    Head | NoScript => return false,
                    _ => {}
                }
                // Other tags close noscript and are reprocessed in-head.
                self.pop_through(self.stack.index_of(NoScript).unwrap());
                self.mode = InsertionMode::InHead;
            }
            _ => {}
        }
        match tag.id() {
            Html | Head | Base | BaseFont | BgSound | Link | Meta | Title | NoFrames | Style
            | Script => true,
            NoScript => {
                if !self.scripting_enabled {
                    self.mode = InsertionMode::InHeadNoScript;
                }
                true
            }
            Unknown if name_is(tag, "template") => true,
            _ => {
                self.mode = InsertionMode::AfterHead;
                true
            }
        }
    }

    fn push_html(&mut self, tag: &HtmlTagToken) {
        self.stack.push(
            tag.name(),
            tag.id(),
            HtmlNamespace::Html,
            html_flags(tag.id(), tag.name()),
        );
    }

    fn push_implied(&mut self, id: HtmlTagId) {
        self.stack
            .push(id.as_str(), id, HtmlNamespace::Html, html_flags(id, ""));
    }

    fn close_in_scope(&mut self, index: Option<usize>, predicate: Predicate) {
        if in_scope(index, self.stack.nearest(predicate)) {
            self.pop_through(index.unwrap());
        }
    }

    fn close_p(&mut self) {
        self.close_in_scope(self.stack.index_of(HtmlTagId::P), Predicate::ButtonScope);
    }

    fn close_list_item(&mut self, index: Option<usize>) {
        // Only address, div and p may be crossed before reaching the item.
        self.close_in_scope(index, Predicate::ListItemStop);
    }

    fn current_table(&self) -> Option<usize> {
        self.stack
            .index_of(HtmlTagId::Table)
            .filter(|&i| Some(i) == self.stack.nearest(Predicate::TableScope))
    }

    fn process_html_start(
        &mut self,
        tag: &HtmlTagToken,
        entry_mode: InsertionMode,
        html_existed: bool,
    ) {
        use HtmlTagId::*;
        let had_body = self.body_created;
        let id = tag.id();
        // After-head tags outside the head set imply body, except inside a template.
        if !self.body_created
            && self.mode == InsertionMode::AfterHead
            && inserts_body(tag)
            && !self.is_template_open()
        {
            if id == Body {
                self.body_created = true;
            } else {
                self.insert_implied_body();
            }
        }
        if self.frameset_ok && clears_frameset_ok(tag) {
            self.frameset_ok = false;
        }
        match id {
            Html | Body => {
                // Neither html nor body is pushed twice. Later attributes can merge,
                // except inside a template, where the start tag is ignored.
                if self.is_template_open() {
                    self.disposition = HtmlTagDisposition::Ignored;
                } else if (if id == Html { html_existed } else { had_body })
                    && !tag.attributes().is_empty()
                {
                    self.disposition = HtmlTagDisposition::AttributesMerged;
                }
            }
            Head => {
                if entry_mode != InsertionMode::BeforeHead {
                    self.disposition = HtmlTagDisposition::Ignored;
                }
            }
            Frame => self.disposition = HtmlTagDisposition::Ignored,
            FrameSet => {
                // Before body, frameset is accepted regardless of frameset-ok.
                // Otherwise it replaces body only while allowed and outside templates.
                if (self.frameset_ok || !self.body_created) && !self.is_template_open() {
                    self.pop_above(None);
                    self.in_frameset = true;
                    self.frameset_depth = 1;
                } else {
                    self.disposition = HtmlTagDisposition::Ignored;
                }
            }
            Address | Article | Aside | BlockQuote | Center | Details | Dialog | Dir | Div | Dl
            | FieldSet | FigCaption | Figure | Footer | Header | Main | Menu | Nav | Ol | P
            | Section | Summary | Ul | Pre | Listing | PlainText | Xmp => {
                self.close_p();
                self.push_html(tag);
            }
            Form => self.process_form_start(tag),
            H1 | H2 | H3 | H4 | H5 | H6 => {
                self.close_p();
                if self
                    .stack
                    .entries
                    .last()
                    .is_some_and(|e| e.namespace == HtmlNamespace::Html && is_heading(e.id))
                {
                    self.pop_through(self.stack.top().unwrap());
                }
                self.push_html(tag);
            }
            Hr => self.close_p(),
            Li => {
                self.close_list_item(self.stack.index_of(Li));
                self.close_p();
                self.push_html(tag);
            }
            Dd | Dt => {
                self.close_list_item(self.stack.index_of(Dd).max(self.stack.index_of(Dt)));
                self.close_p();
                self.push_html(tag);
            }
            Button => {
                self.close_in_scope(self.stack.index_of(Button), Predicate::Scope);
                self.push_html(tag);
            }
            A => {
                // Approximate adoption for nested anchors when no special element intervenes.
                if let Some(i) = self.stack.index_of(A)
                    && self.stack.nearest(Predicate::Special) < Some(i)
                {
                    self.pop_through(i);
                }
                self.push_html(tag);
            }
            Option | OptGroup => {
                if self.stack.is_top_html(Option) {
                    self.pop_through(self.stack.top().unwrap());
                }
                self.push_html(tag);
            }
            Table => {
                if let Some(table) = self.current_table()
                    && self
                        .stack
                        .index_of(Td)
                        .max(self.stack.index_of(Th))
                        .max(self.stack.index_of(Caption))
                        < Some(table)
                {
                    // In table/table-body/row modes, a new table closes the current one.
                    self.pop_through(table);
                } else {
                    self.close_p();
                }
                self.push_html(tag);
            }
            Caption | ColGroup | Col | TBody | THead | Tfoot | Tr | Td | Th => {
                self.process_table_start(tag)
            }
            Unknown if name_is(tag, "svg") || name_is(tag, "math") => {
                if !tag.is_empty_element() {
                    let ns = if name_is(tag, "svg") {
                        HtmlNamespace::Svg
                    } else {
                        HtmlNamespace::MathMl
                    };
                    self.stack.push(tag.name(), Unknown, ns, 0);
                }
            }
            Unknown if name_is(tag, "hgroup") || name_is(tag, "search") => {
                self.close_p();
                self.push_html(tag);
            }
            _ => {
                // HTML non-void elements ignore the self-closing flag.
                if !is_void(id, tag.name()) {
                    self.push_html(tag);
                }
            }
        }
    }

    fn process_form_start(&mut self, tag: &HtmlTagToken) {
        use HtmlTagId::*;
        let template = self.is_template_open();
        // A form pointer remains set even if its element was already popped.
        if self.form_pointer_set && !template {
            self.disposition = HtmlTagDisposition::Ignored;
            return;
        }
        if self.stack.entries.last().is_some_and(|e| {
            e.namespace == HtmlNamespace::Html && matches!(e.id, Table | TBody | Tfoot | THead | Tr)
        }) {
            // In table/table-body/row modes, form is inserted then immediately
            // popped, or ignored if a template is open.
            if !template {
                self.form_pointer_set = true;
            } else {
                self.disposition = HtmlTagDisposition::Ignored;
            }
            return;
        }
        self.close_p();
        self.push_html(tag);
        if !template {
            self.form_pointer_set = true;
            self.form_pointer_index = self.stack.top().filter(|_| self.stack.is_top_html(Form));
        }
    }

    fn process_table_start(&mut self, tag: &HtmlTagToken) {
        use HtmlTagId::*;
        let Some(table) = self.current_table() else {
            // Table-structure tags outside a table are ignored.
            if self.stack.nearest(Predicate::TableScope).is_none() {
                self.disposition = HtmlTagDisposition::Ignored;
            }
            return;
        };
        match tag.id() {
            Caption | ColGroup | TBody | THead | Tfoot => {
                self.pop_above(Some(table));
                self.push_html(tag);
            }
            Col => {
                self.pop_above(Some(table));
                self.push_implied(ColGroup);
            }
            Tr => {
                self.ensure_table_body(table);
                self.push_html(tag);
            }
            _ => {
                if let Some(row) = self.stack.index_of(Tr).filter(|&i| i > table) {
                    self.pop_above(Some(row));
                } else {
                    self.ensure_table_body(table);
                    self.push_implied(Tr);
                }
                self.push_html(tag);
            }
        }
    }

    fn ensure_table_body(&mut self, table: usize) {
        use HtmlTagId::*;
        let section = self
            .stack
            .index_of(TBody)
            .max(self.stack.index_of(THead))
            .max(self.stack.index_of(Tfoot));
        if section > Some(table) {
            self.pop_above(section);
        } else {
            self.pop_above(Some(table));
            self.push_implied(TBody);
        }
    }

    fn process_end_tag(&mut self, tag: &HtmlTagToken) {
        use HtmlTagId::*;
        self.popped_through = false;
        // Only these end tags imply html in the before-html insertion mode.
        if matches!(tag.id(), Head | Body | Html | Br) {
            self.html_created = true;
        }
        if self.in_frameset {
            if tag.id() == FrameSet {
                self.frameset_depth = self.frameset_depth.saturating_sub(1);
            }
            return;
        }
        if self.mode != InsertionMode::AfterHead {
            self.process_head_end(tag);
        }
        if !self.body_created
            && self.mode == InsertionMode::AfterHead
            && matches!(tag.id(), Body | Html | Br)
            && !self.is_template_open()
        {
            self.insert_implied_body();
        }
        // In-body </br> is treated as a br start tag.
        if tag.id() == Br {
            self.frameset_ok = false;
        }
        self.process_end_internal(tag);
        // html/head/body/br never close a stack entry, but are not considered ignored.
        if !self.popped_through
            && !matches!(tag.id(), Html | Head | Body | Br)
            && (self
                .stack
                .index_of_name(HtmlNamespace::Html, tag.id(), tag.name())
                .is_some()
                || self
                    .stack
                    .index_of_name(HtmlNamespace::Svg, Unknown, tag.name())
                    .is_some())
        {
            self.disposition = HtmlTagDisposition::Ignored;
        }
    }

    fn process_head_end(&mut self, tag: &HtmlTagToken) {
        use HtmlTagId::*;
        if self.is_template_open() {
            return;
        }
        match tag.id() {
            NoScript if self.mode == InsertionMode::InHeadNoScript => {
                self.mode = InsertionMode::InHead
            }
            Br => {
                if self.mode == InsertionMode::InHeadNoScript {
                    self.pop_through(self.stack.index_of(NoScript).unwrap());
                }
                self.mode = InsertionMode::AfterHead;
            }
            // These are ignored while in-head-noscript.
            Head | Body | Html if self.mode != InsertionMode::InHeadNoScript => {
                self.mode = InsertionMode::AfterHead
            }
            _ => {}
        }
    }

    fn process_end_internal(&mut self, tag: &HtmlTagToken) {
        if self.is_in_foreign_content() {
            if matches!(tag.id(), HtmlTagId::Br | HtmlTagId::P) {
                let count = self.stack.entries.len();
                self.pop_above(self.stack.nearest(Predicate::HtmlOrIntegrationPoint));
                self.broke_out = self.stack.entries.len() < count;
            } else {
                // Search foreign names down to the first HTML element. A miss
                // is reprocessed using HTML rules, not silently discarded.
                let index =
                    self.stack
                        .index_of_name(HtmlNamespace::Svg, HtmlTagId::Unknown, tag.name());
                if index > self.stack.nearest(Predicate::Html) {
                    self.pop_through(index.unwrap());
                    return;
                }
            }
        }
        self.process_html_end(tag);
    }

    fn process_form_end(&mut self) {
        let index = self.stack.index_of(HtmlTagId::Form);
        if self.is_template_open() {
            self.close_in_scope(index, Predicate::Scope);
            return;
        }
        // Unset the pointer and remove its form without popping elements above
        // it: <form><svg><style></form> must retain the foreign elements.
        let index = if self.form_pointer_set {
            self.form_pointer_index
        } else {
            None
        };
        self.form_pointer_set = false;
        self.form_pointer_index = None;
        if !in_scope(index, self.stack.nearest(Predicate::Scope)) {
            return;
        }
        let index = index.unwrap();
        let mut top = self.stack.top();
        while let Some(i) = top
            && self.stack.entries[i].namespace == HtmlNamespace::Html
            && implied_end_tag(&self.stack.entries[i])
        {
            top = i.checked_sub(1);
        }
        self.pop_above(top);
        if Some(index) == self.stack.top() {
            self.pop_through(index);
        } else {
            self.popped_through = true;
            self.stack.remove_entry(index);
        }
    }

    fn process_formatting_end(&mut self, id: HtmlTagId) {
        let index = self.stack.index_of(id);
        if !in_scope(index, self.stack.nearest(Predicate::Scope)) {
            return;
        }
        let mut index = index.unwrap();
        // Model stack effects, not DOM reparenting or active-formatting reconstruction.
        for _ in 0..8 {
            if self.stack.nearest(Predicate::Special) < Some(index) {
                self.pop_through(index);
                return;
            }
            let mut block = index + 1;
            while self.stack.entries[block].flags & SPECIAL == 0 {
                block += 1;
            }
            let formatting = self.stack.entries[index].clone();
            let furthest = self.stack.entries[block].clone();
            let mut replacement = Vec::with_capacity(7);
            // Only the last physical formatting node moves; only the first
            // special node in a folded run is the furthest block.
            if formatting.count > 1 {
                let mut entry = formatting.clone();
                entry.count -= 1;
                replacement.push(entry);
            }
            let first = replacement.len();
            let mut remaining = 3;
            for i in (index + 1..block).rev() {
                if remaining == 0 {
                    break;
                }
                let mut node = self.stack.entries[i].clone();
                let retained = remaining.min(node.count);
                remaining -= retained;
                if node.namespace == HtmlNamespace::Html && node.id.is_formatting_element() {
                    node.count = retained;
                    replacement.push(node);
                }
            }
            // The inner walk is backwards but retained nodes keep their stack order.
            replacement[first..].reverse();
            let mut node = furthest.clone();
            node.count = 1;
            replacement.push(node);
            let mut node = formatting;
            node.count = 1;
            replacement.push(node);
            if furthest.count > 1 {
                let mut node = furthest;
                node.count -= 1;
                replacement.push(node);
            }
            // Remove all discarded inner nodes together instead of rebuilding
            // the same suffix repeatedly for a long inner walk.
            self.stack
                .replace_range(index, block - index + 1, &replacement);
            self.popped_through = true;
            if self.form_pointer_index >= Some(index) {
                self.form_pointer_index = self.stack.index_of(HtmlTagId::Form);
            }
            if self.depth_exceeded() {
                return;
            }
            index = self.stack.index_of(id).unwrap();
        }
    }

    fn process_html_end(&mut self, tag: &HtmlTagToken) {
        use HtmlTagId::*;
        match tag.id() {
            Html | Head | Body | Br => {}
            P => self.close_in_scope(self.stack.index_of(P), Predicate::ButtonScope),
            Li => self.close_in_scope(self.stack.index_of(Li), Predicate::ListItemScope),
            // Any heading end tag closes the topmost in-scope heading.
            H1 | H2 | H3 | H4 | H5 | H6 => {
                self.close_in_scope(self.stack.nearest(Predicate::Heading), Predicate::Scope)
            }
            Address | Article | Aside | BlockQuote | Button | Center | Details | Dialog | Dir
            | Div | Dl | FieldSet | FigCaption | Figure | Footer | Header | Listing | Main
            | Menu | Nav | Ol | Pre | Section | Summary | Ul | Applet | Marquee | Object | Dd
            | Dt => self.close_in_scope(self.stack.index_of(tag.id()), Predicate::Scope),
            Table | TBody | Tfoot | THead | Tr | Td | Th | Caption | ColGroup => {
                self.close_in_scope(self.stack.index_of(tag.id()), Predicate::TableScope)
            }
            Form => self.process_form_end(),
            A | B | Big | Code | Em | Font | I | NoBr | S | Small | Strike | Strong | Tt | U => {
                self.process_formatting_end(tag.id())
            }
            Unknown if name_is(tag, "hgroup") || name_is(tag, "search") => self.close_in_scope(
                self.stack
                    .index_of_name(HtmlNamespace::Html, Unknown, tag.name()),
                Predicate::Scope,
            ),
            Unknown if name_is(tag, "template") => {
                if let Some(index) =
                    self.stack
                        .index_of_name(HtmlNamespace::Html, Unknown, tag.name())
                {
                    self.pop_through(index);
                }
            }
            _ => {
                // Other end tags cannot cross a special element to reach their match.
                let index = self
                    .stack
                    .index_of_name(HtmlNamespace::Html, tag.id(), tag.name());
                self.close_in_scope(index, Predicate::Special);
            }
        }
    }
}

fn in_scope(index: Option<usize>, boundary: Option<usize>) -> bool {
    index.is_some() && index >= boundary
}
fn name_is(tag: &HtmlTagToken, name: &str) -> bool {
    tag.name().eq_ignore_ascii_case(name)
}
fn is_heading(id: HtmlTagId) -> bool {
    matches!(
        id,
        HtmlTagId::H1
            | HtmlTagId::H2
            | HtmlTagId::H3
            | HtmlTagId::H4
            | HtmlTagId::H5
            | HtmlTagId::H6
    )
}

fn implied_end_tag(entry: &Entry) -> bool {
    use HtmlTagId::*;
    matches!(entry.id, Dd | Dt | Li | OptGroup | Option | P | Rp | Rt)
        || (entry.id == Unknown
            && (entry.name.eq_ignore_ascii_case("rb") || entry.name.eq_ignore_ascii_case("rtc")))
}

fn html_flags(id: HtmlTagId, name: &str) -> u16 {
    use HtmlTagId::*;
    match id {
        Applet | Caption | Marquee | Object | Td | Th => SPECIAL | SCOPE | LIST_ITEM_STOP,
        Html | Table => SPECIAL | SCOPE | TABLE_SCOPE | LIST_ITEM_STOP,
        Ol | Ul => SPECIAL | LIST_ITEM_SCOPE | LIST_ITEM_STOP,
        Button => SPECIAL | BUTTON_SCOPE | LIST_ITEM_STOP,
        H1 | H2 | H3 | H4 | H5 | H6 => SPECIAL | HEADING | LIST_ITEM_STOP,
        Address | Div | P => SPECIAL,
        Area | Article | Aside | Base | BaseFont | BgSound | BlockQuote | Body | Br | Center
        | Col | ColGroup | Dd | Details | Dir | Dl | Dt | Embed | FieldSet | FigCaption
        | Figure | Footer | Form | Frame | FrameSet | Head | Header | Hr | IFrame | Image
        | Input | Keygen | Li | Link | Listing | Main | Menu | Meta | Nav | NoEmbed | NoFrames
        | NoScript | Param | PlainText | Pre | Script | Section | Select | Source | Style
        | Summary | TBody | TextArea | Tfoot | THead | Title | Tr | Track | Wbr | Xmp => {
            SPECIAL | LIST_ITEM_STOP
        }
        Unknown if name.eq_ignore_ascii_case("template") => {
            SPECIAL | SCOPE | TABLE_SCOPE | LIST_ITEM_STOP
        }
        Unknown if name.eq_ignore_ascii_case("hgroup") || name.eq_ignore_ascii_case("search") => {
            SPECIAL | LIST_ITEM_STOP
        }
        _ => 0,
    }
}

fn foreign_flags(ns: HtmlNamespace, tag: &HtmlTagToken) -> u16 {
    const INTEGRATION: u16 = SPECIAL | SCOPE | LIST_ITEM_STOP;
    if ns == HtmlNamespace::MathMl {
        if ["mi", "mo", "mn", "ms", "mtext"]
            .iter()
            .any(|name| name_is(tag, name))
        {
            return INTEGRATION | MATH_INTEGRATION;
        }
        if name_is(tag, "annotation-xml") {
            let html = tag
                .attributes()
                .get("encoding")
                .and_then(|a| a.value())
                .is_some_and(|v| {
                    v.eq_ignore_ascii_case("text/html")
                        || v.eq_ignore_ascii_case("application/xhtml+xml")
                });
            return INTEGRATION | if html { HTML_INTEGRATION } else { 0 };
        }
    } else if ns == HtmlNamespace::Svg
        && ["foreignObject", "desc", "title"]
            .iter()
            .any(|name| name_is(tag, name))
    {
        return INTEGRATION | HTML_INTEGRATION;
    }
    0
}

fn foreign_breakout(tag: &HtmlTagToken) -> bool {
    use HtmlTagId::*;
    match tag.id() {
        B | Big | BlockQuote | Body | Br | Center | Code | Dd | Div | Dl | Dt | Em | Embed | H1
        | H2 | H3 | H4 | H5 | H6 | Head | Hr | I | Image | Li | Listing | Menu | Meta | NoBr
        | Ol | P | Pre | Ruby | S | Small | Span | Strong | Strike | Sub | Sup | Table | Tt | U
        | Ul | Var => true,
        Font => tag.attributes().iter().any(|a| {
            matches!(
                a.id(),
                HtmlAttributeId::Color | HtmlAttributeId::Face | HtmlAttributeId::Size
            )
        }),
        _ => false,
    }
}

fn inserts_body(tag: &HtmlTagToken) -> bool {
    use HtmlTagId::*;
    match tag.id() {
        Html | Head | FrameSet | Base | BaseFont | BgSound | Link | Meta | NoFrames | Script
        | Style | Title => false,
        Unknown => !name_is(tag, "template"),
        _ => true,
    }
}

fn clears_frameset_ok(tag: &HtmlTagToken) -> bool {
    use HtmlTagId::*;
    match tag.id() {
        Pre | Listing | Li | Dd | Dt | Button | Applet | Marquee | Object | Table | Area | Br
        | Embed | Image | Keygen | Wbr | Hr | TextArea | Xmp | IFrame | Select | Body => true,
        Input => {
            // Only the first type attribute counts. Fold ASCII only:
            // non-ASCII lookalikes are not the hidden input type.
            !tag.attributes()
                .get_by_id(HtmlAttributeId::Type)
                .and_then(|a| a.value())
                .is_some_and(|v| v.eq_ignore_ascii_case("hidden"))
        }
        Unknown => name_is(tag, "image") || name_is(tag, "template"),
        _ => false,
    }
}

fn is_void(id: HtmlTagId, name: &str) -> bool {
    matches!(
        id,
        HtmlTagId::BaseFont | HtmlTagId::BgSound | HtmlTagId::Frame
    ) || (id == HtmlTagId::Unknown && name.eq_ignore_ascii_case("image"))
        || id.is_empty_element()
}

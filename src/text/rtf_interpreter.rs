// Adapted from MimeKit's Text/RtfInterpreter.cs and Text/RtfToText.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::collections::HashMap;
use std::io::{self, BufRead};
use std::num::NonZeroUsize;
use std::rc::Rc;

use super::rtf_reporting::{RtfReport, RtfReportingLimits};
use super::{
    RtfControlWord, RtfIssue, RtfKeyword as K, RtfSourcePosition, RtfToken, RtfTokenKind,
    RtfValidator, RtfViolation as V, parse_rtf_hyperlink,
};
use crate::charsets::{Charset, from_codepage};
use crate::filters::CharsetFilter;

pub(super) const DEFAULT_LIMIT: NonZeroUsize = NonZeroUsize::new(4096).unwrap();
const MAX_CODE_PAGES: usize = 32;
const BYTE_BUFFER_SIZE: usize = 1024;
pub(super) const MAX_FIELD_INSTRUCTION_LENGTH: usize = 2048;
pub(super) const BOLD: u16 = 1 << 0;
pub(super) const ITALIC: u16 = 1 << 1;
pub(super) const UNDERLINE: u16 = 1 << 2;
pub(super) const STRIKE: u16 = 1 << 3;
const HIDDEN: u16 = 1 << 4;
pub(super) const IN_TABLE: u16 = 1 << 5;
const HTML_RTF: u16 = 1 << 6;
const CHARACTER_FORMATTING: u16 = BOLD | ITALIC | UNDERLINE | STRIKE | HIDDEN;

#[derive(Clone, Copy)]
pub(super) struct InterpreterOptions {
    pub max_group_depth: NonZeroUsize,
    pub max_font_table_entries: NonZeroUsize,
    pub max_color_table_entries: Option<NonZeroUsize>,
    pub collect_hyperlinks: bool,
    pub extract_html: bool,
    pub max_issues_per_violation: usize,
}

pub(super) enum Content<'a> {
    Text(&'a str),
    Paragraph,
    LineBreak,
    Cell,
    Row,
    ObjectPlaceholder(usize),
}

pub(super) trait ContentHandler {
    fn begin(&mut self, _from_html: bool) {}

    fn content(&mut self, state: &GroupState, colors: &[Option<u32>], content: Content<'_>);
}

pub(super) fn interpret<R: BufRead, F: FnMut(RtfIssue), H: ContentHandler>(
    reader: R,
    options: InterpreterOptions,
    report: Option<F>,
    handler: H,
) -> io::Result<H> {
    // The underlying validator is unlimited: all diagnostics pass through the
    // interpreter's single reporting budget, irrespective of rendering mode.
    let mut validator = RtfValidator::new(reader);
    let mut interpreter = Interpreter::new(options, report, handler);
    loop {
        let token = if interpreter.report.is_some() {
            validator.read_token_with_issues(|issue| interpreter.report_issue(issue))?
        } else {
            validator.read_token()?
        };
        let Some(token) = token else { break };
        interpreter.process(token);
    }
    interpreter.position = validator.source_position();
    interpreter.flush_bytes();
    interpreter.flush_high_surrogate();
    interpreter.begin();
    Ok(interpreter.handler)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Destination {
    #[default]
    Normal,
    FontTable,
    ColorTable,
    FieldInstruction,
    HtmlTag,
    // {\upr {ansi} {\*\ud {unicode}}}: suppress the ANSI alternative.
    Upr,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) enum VerticalAlignment {
    #[default]
    Baseline,
    Superscript,
    Subscript,
}

// Group-scoped properties are saved on '{' and restored on '}'. A small value
// keeps pushes and equality checks O(1), including otherwise invisible styles
// that distinguish nested states for the group resource limit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct GroupState {
    destination: Destination,
    pub alignment: Alignment,
    pub vertical_alignment: VerticalAlignment,
    pub flags: u16,
    font: i32,
    pub font_size: i32,
    pub foreground: i32,
    pub background: i32,
    unicode_skip: usize,
    pub hyperlink: Option<Hyperlink>,
}

impl Default for GroupState {
    fn default() -> Self {
        Self {
            destination: Destination::Normal,
            alignment: Alignment::Left,
            vertical_alignment: VerticalAlignment::Baseline,
            flags: 0,
            font: 0,
            // RTF defaults: \fs24 is 12pt, and \uc1 skips one fallback character.
            font_size: 24,
            unicode_skip: 1,
            hyperlink: None,
            foreground: 0,
            background: 0,
        }
    }
}

impl GroupState {
    fn set_flag(&mut self, flag: u16, value: bool) {
        if value {
            self.flags |= flag
        } else {
            self.flags &= !flag
        }
    }
}

struct StackEntry {
    state: GroupState,
    // Repeat counts must not participate in formatting-state equality.
    count: usize,
}

struct PendingFont {
    number: i32,
    charset: i32,
    code_page: i32,
    depth: usize,
}

// Links belong to result-group state, not to one global "current link".
// Reference identity keeps stack folding O(1), even for long nested targets.
#[derive(Clone, Debug)]
pub(super) struct Hyperlink(Rc<str>);

impl Hyperlink {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl PartialEq for Hyperlink {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Hyperlink {}

struct Interpreter<F, H> {
    options: InterpreterOptions,
    handler: H,
    report: Option<F>,
    reporting: RtfReportingLimits<{ V::TooManyComplianceIssues as usize }>,
    position: RtfSourcePosition,
    current: GroupState,
    stack: Vec<StackEntry>,
    depth: usize,
    root_closed: bool,
    skip_depth: usize,
    skip_pops_group: bool,
    ignorable_destination: bool,
    unicode_skip_remaining: usize,
    high_surrogate: Option<u16>,
    object_placeholder_pending: bool,
    placeholder_index: usize,
    document_code_page: i32,
    default_font: i32,
    fonts: HashMap<i32, i32>,
    pending_font: Option<PendingFont>,
    colors: Vec<Option<u32>>,
    pending_color: Option<[u8; 3]>,
    field_instruction: String,
    field_hyperlink: Option<Hyperlink>,
    reported_field_limit: bool,
    reported_color_limit: bool,
    code_pages: HashMap<i32, Charset>,
    decoder: Option<CharsetFilter>,
    bytes: [u8; BYTE_BUFFER_SIZE],
    byte_count: usize,
    bytes_code_page: i32,
    decoded: Vec<u8>,
    reported_decode_error: bool,
    reported_group_limit: bool,
    reported_font_limit: bool,
    reported_code_page_limit: bool,
    reported_eight_bit: bool,
    from_html: bool,
    recognition_tokens: u8,
    recognition_window_open: bool,
}

impl<F: FnMut(RtfIssue), H: ContentHandler> Interpreter<F, H> {
    fn new(options: InterpreterOptions, report: Option<F>, handler: H) -> Self {
        Self {
            options,
            handler,
            report,
            reporting: RtfReportingLimits::new(options.max_issues_per_violation),
            position: RtfSourcePosition {
                offset: 0,
                line_number: 1,
                line_position: 1,
                depth: 0,
            },
            current: GroupState::default(),
            stack: Vec::new(),
            depth: 0,
            root_closed: false,
            skip_depth: 0,
            skip_pops_group: false,
            ignorable_destination: false,
            unicode_skip_remaining: 0,
            high_surrogate: None,
            object_placeholder_pending: false,
            placeholder_index: 0,
            // RTF's \ansi default is Windows-1252, independent of host locale.
            document_code_page: 1252,
            default_font: 0,
            fonts: HashMap::new(),
            pending_font: None,
            colors: Vec::new(),
            pending_color: None,
            field_instruction: String::new(),
            field_hyperlink: None,
            reported_field_limit: false,
            reported_color_limit: false,
            code_pages: HashMap::new(),
            decoder: None,
            bytes: [0; BYTE_BUFFER_SIZE],
            byte_count: 0,
            bytes_code_page: 0,
            decoded: Vec::new(),
            reported_decode_error: false,
            reported_group_limit: false,
            reported_font_limit: false,
            reported_code_page_limit: false,
            reported_eight_bit: false,
            from_html: false,
            recognition_tokens: 0,
            recognition_window_open: true,
        }
    }

    fn report_issue(&mut self, mut issue: RtfIssue) {
        let Some(report) = &mut self.report else {
            return;
        };
        match self.reporting.check(issue.violation as usize) {
            RtfReport::Issue => {}
            RtfReport::Limit => issue.violation = V::TooManyComplianceIssues,
            RtfReport::Suppress => return,
        }
        report(issue);
    }

    fn report_at(&mut self, violation: V, delta: usize) {
        let mut position = self.position;
        position.offset += delta as u64;
        position.line_position = position.line_position.saturating_add(delta as u64);
        self.report_issue(RtfIssue {
            violation,
            position,
        });
    }

    fn begin(&mut self) {
        if self.recognition_window_open {
            self.recognition_window_open = false;
            self.handler.begin(self.from_html);
        }
    }

    fn extracting_html(&self) -> bool {
        self.options.extract_html && self.from_html
    }

    fn process(&mut self, token: RtfToken) {
        self.position = token.position;
        if self.recognition_window_open {
            // The recognition prefix contains at most ten group-start/control-
            // word tokens. Decide before processing the token that ends it,
            // including tokens in destinations that will subsequently be skipped.
            self.recognition_tokens += 1;
            if self.recognition_tokens > 10
                || !matches!(
                    token.kind,
                    RtfTokenKind::GroupStart | RtfTokenKind::ControlWord(_)
                )
            {
                self.begin();
            }
        }
        if !matches!(token.kind, RtfTokenKind::Text(_)) {
            self.reported_eight_bit = false;
        }
        let replace_placeholder = std::mem::take(&mut self.object_placeholder_pending);

        // Only braces matter inside a skipped destination. The tokenizer has
        // already removed \bin payloads, so even huge skipped trees need O(1)
        // storage and binary braces cannot disturb the skip counter.
        if self.skip_depth > 0 {
            match token.kind {
                RtfTokenKind::GroupStart => self.skip_depth = self.skip_depth.saturating_add(1),
                RtfTokenKind::GroupEnd => {
                    self.skip_depth -= 1;
                    if self.skip_depth == 0 && self.skip_pops_group {
                        self.pop_group();
                    }
                }
                _ => {}
            }
            return;
        }

        match token.kind {
            RtfTokenKind::Text(bytes) => {
                self.ignorable_destination = false;
                self.process_text(&bytes, replace_placeholder);
                return;
            }
            RtfTokenKind::HexChar(byte) => {
                self.ignorable_destination = false;
                if !(replace_placeholder && byte == b' ') {
                    if self.unicode_skip_remaining > 0 {
                        self.unicode_skip_remaining -= 1;
                    } else if self.current.destination != Destination::Upr {
                        self.append_byte(byte);
                    }
                }
                return;
            }
            RtfTokenKind::ControlSymbol(symbol) => {
                self.process_symbol(symbol);
                return;
            }
            _ => {}
        }

        // Text, hex escapes, and literal escape symbols can share DBCS bytes.
        // Every other token ends the sequence before changing formatting.
        self.flush_bytes();
        if !matches!(&token.kind, RtfTokenKind::ControlWord(word) if word.id() == K::U) {
            self.flush_high_surrogate();
        }
        match token.kind {
            RtfTokenKind::GroupStart => {
                // A group boundary ends the Unicode ANSI-fallback run.
                self.ignorable_destination = false;
                self.unicode_skip_remaining = 0;
                self.push_group();
            }
            RtfTokenKind::GroupEnd => {
                self.ignorable_destination = false;
                self.unicode_skip_remaining = 0;
                self.pop_group();
            }
            RtfTokenKind::ControlWord(word) => self.process_word(word),
            _ => unreachable!("text and escapes were handled above"),
        }
    }

    fn push_group(&mut self) {
        // Consecutive identical saved states share one entry. A million nested
        // braces without formatting changes therefore takes constant space.
        if let Some(top) = self.stack.last_mut()
            && top.state == self.current
        {
            top.count = top.count.saturating_add(1);
            self.depth = self.depth.saturating_add(1);
            return;
        }
        let limit = self.options.max_group_depth.get();
        if self.stack.len() == limit {
            // Nothing was pushed: the closing brace must not pop a saved
            // state. Discarding content beats rendering it with wrong state.
            self.skip_depth = 1;
            self.skip_pops_group = false;
            if !self.reported_group_limit {
                self.reported_group_limit = true;
                self.report_at(V::GroupDepthLimitExceeded, 0);
            }
            return;
        }
        // Grow on demand instead of reserving the configured maximum up front.
        if self.stack.len() == self.stack.capacity() {
            let capacity = self.stack.capacity().saturating_mul(2).max(16).min(limit);
            self.stack.reserve_exact(capacity - self.stack.len());
        }
        self.stack.push(StackEntry {
            state: self.current.clone(),
            count: 1,
        });
        self.depth = self.depth.saturating_add(1);
    }

    fn pop_group(&mut self) {
        let Some(top) = self.stack.last_mut() else {
            // The validator already reported this unmatched brace. Ignore it
            // rather than failing the rest of a malformed document.
            return;
        };
        let previous_destination = self.current.destination;
        self.current = top.state.clone();
        top.count -= 1;
        if top.count == 0 {
            self.stack.pop();
        }
        self.depth -= 1;
        self.root_closed |= self.depth == 0;
        // A font may end without ';', but a nested \panose or \falt subgroup
        // must not commit it before later \fcharset or \cpg words are seen.
        if self
            .pending_font
            .as_ref()
            .is_some_and(|font| self.depth < font.depth)
        {
            self.commit_font(0);
        }
        // The instruction is complete when leaving \fldinst, before its
        // sibling \fldrslt begins. Nested result groups restore their own link.
        if self.options.collect_hyperlinks
            && previous_destination == Destination::FieldInstruction
            && self.current.destination != Destination::FieldInstruction
        {
            self.field_hyperlink =
                parse_rtf_hyperlink(&self.field_instruction).map(|link| Hyperlink(Rc::from(link)));
        }
    }

    fn skip_current_group(&mut self) {
        // Never discard the initial root because of a misplaced destination.
        // After an early root close, later groups may still need skipping.
        if self.depth <= 1 && !self.root_closed {
            return;
        }
        self.skip_depth = 1;
        self.skip_pops_group = true;
    }

    fn commit_font(&mut self, delta: usize) {
        let Some(font) = self.pending_font.take() else {
            return;
        };
        // Full tables can update known numbers, but cannot add new ones.
        if self.fonts.len() == self.options.max_font_table_entries.get()
            && !self.fonts.contains_key(&font.number)
        {
            if !self.reported_font_limit {
                self.reported_font_limit = true;
                self.report_at(V::FontTableLimitExceeded, delta);
            }
            return;
        }
        // An explicit \cpg overrides the code page implied by \fcharset.
        let code_page = if font.code_page > 0 {
            font.code_page
        } else {
            font_code_page(font.charset)
        };
        self.fonts.insert(font.number, code_page);
    }

    fn commit_color(&mut self, delta: usize) {
        let Some(limit) = self.options.max_color_table_entries else {
            return;
        };
        // A bare ';' denotes the automatic color. Excess entries are dropped;
        // references to them also render using the automatic color.
        let color = self
            .pending_color
            .take()
            .map(|[r, g, b]| (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b));
        if self.colors.len() < limit.get() {
            if self.colors.len() == self.colors.capacity() {
                let capacity = self
                    .colors
                    .capacity()
                    .saturating_mul(2)
                    .max(16)
                    .min(limit.get());
                self.colors.reserve_exact(capacity - self.colors.len());
            }
            self.colors.push(color);
        } else if !self.reported_color_limit {
            self.reported_color_limit = true;
            self.report_at(V::ColorTableLimitExceeded, delta);
        }
    }

    fn collect_instruction(&mut self, text: &str) {
        if self.reported_field_limit {
            return;
        }
        // Retain a bounded prefix, in UTF-8 bytes rather than UTF-16 units.
        // Never split a scalar or append later text after dropping a character.
        let available = MAX_FIELD_INSTRUCTION_LENGTH - self.field_instruction.len();
        let mut count = text.len().min(available);
        while !text.is_char_boundary(count) {
            count -= 1;
        }
        if count < text.len() {
            self.reported_field_limit = true;
            self.report_at(V::FieldInstructionTooLong, 0);
        }
        if count > 0 {
            if self.field_instruction.capacity() == 0 {
                self.field_instruction
                    .reserve_exact(MAX_FIELD_INSTRUCTION_LENGTH);
            }
            self.field_instruction.push_str(&text[..count]);
        }
    }

    fn current_code_page(&self) -> i32 {
        // HTMLTAG contents always use the document code page, regardless of
        // the current font ([MS-OXRTFEX] 2.2.3.2).
        if self.current.destination == Destination::HtmlTag {
            return self.document_code_page;
        }
        self.fonts
            .get(&self.current.font)
            .copied()
            .filter(|&page| page > 0)
            .unwrap_or(self.document_code_page)
    }

    fn resolve_code_page(&mut self) -> Charset {
        if let Some(&charset) = self.code_pages.get(&self.bytes_code_page) {
            return charset;
        }
        let fallback = from_codepage(1252).expect("built-in Windows-1252");
        // A hostile font table can request a new page for every run. Bound
        // cached resolutions (including failures) and stop resolving at the cap.
        if self.code_pages.len() == MAX_CODE_PAGES {
            if !self.reported_code_page_limit {
                self.reported_code_page_limit = true;
                self.report_at(V::CodePageLimitExceeded, 0);
            }
            return fallback;
        }
        let charset = match u32::try_from(self.bytes_code_page)
            .ok()
            .and_then(from_codepage)
        {
            Some(charset) => charset,
            None => {
                self.report_at(V::UnsupportedCodePage, 0);
                fallback
            }
        };
        self.code_pages.insert(self.bytes_code_page, charset);
        charset
    }

    fn append_byte(&mut self, byte: u8) {
        let code_page = self.current_code_page();
        if code_page != self.bytes_code_page {
            self.flush_bytes();
            self.bytes_code_page = code_page;
        }
        // Decode a full buffer without finishing: a DBCS lead byte may need
        // a trail byte from the next token or buffer.
        if self.byte_count == self.bytes.len() {
            self.decode_bytes(false);
        }
        self.bytes[self.byte_count] = byte;
        self.byte_count += 1;
    }

    fn flush_bytes(&mut self) {
        if self.byte_count != 0 || self.decoder.is_some() {
            self.decode_bytes(true);
        }
    }

    fn decode_bytes(&mut self, finish: bool) {
        if self.decoder.is_none() {
            let charset = self.resolve_code_page();
            self.decoder = Some(CharsetFilter::new(charset, Charset::UTF8));
        }
        let decoder = self.decoder.as_mut().expect("active byte decoder");
        let mut decoded = std::mem::take(&mut self.decoded);
        decoder.filter(&self.bytes[..self.byte_count], &mut decoded);
        self.byte_count = 0;
        if finish {
            // Finishing even with no new bytes resolves pending lead bytes.
            decoder.finish(&mut decoded);
        }
        let errors = decoder.had_decode_errors();
        if errors && !self.reported_decode_error {
            self.reported_decode_error = true;
            self.report_at(V::InvalidByteSequence, 0);
        }
        if finish {
            self.decoder = None;
            self.reported_decode_error = false;
        }
        if !decoded.is_empty() {
            let text = std::str::from_utf8(&decoded).expect("CharsetFilter's UTF-8 output");
            self.emit_text(text);
        }
        decoded.clear();
        self.decoded = decoded;
    }

    fn visible(&self) -> bool {
        self.current.destination == Destination::Normal
            && self.current.flags & HIDDEN == 0
            && !(self.current.flags & HTML_RTF != 0 && self.extracting_html())
    }

    fn deliver(&mut self, content: Content<'_>) {
        // Content-producing control words can end recognition before ten tokens.
        self.begin();
        self.handler.content(&self.current, &self.colors, content);
    }

    fn emit_text(&mut self, text: &str) {
        self.flush_high_surrogate();
        match self.current.destination {
            Destination::Normal if self.visible() => {
                self.deliver(Content::Text(text));
            }
            Destination::HtmlTag => {
                // HTMLTAG is original markup, not RTF-only display content.
                // Like MimeKit, copy it even inside hidden/HTMLRTF state.
                self.deliver(Content::Text(text));
            }
            Destination::FontTable => {
                // Font names are never stored; only entry terminators matter.
                for c in text.chars() {
                    if c == ';' {
                        self.commit_font(0)
                    }
                }
            }
            Destination::ColorTable => {
                for c in text.chars() {
                    if c == ';' {
                        self.commit_color(0);
                    }
                }
            }
            Destination::FieldInstruction if self.options.collect_hyperlinks => {
                self.collect_instruction(text);
            }
            _ => {}
        }
    }

    fn emit_char(&mut self, c: char) {
        self.flush_bytes();
        self.emit_text(c.encode_utf8(&mut [0; 4]));
    }

    fn flush_high_surrogate(&mut self) {
        if self.high_surrogate.take().is_some() {
            // Unpaired UTF-16 cannot be emitted as valid Unicode text.
            self.report_at(V::UnpairedSurrogate, 0);
            self.emit_text("\u{fffd}");
        }
    }

    fn emit_unicode(&mut self, unit: u16) {
        self.flush_bytes();
        match unit {
            0xdc00..=0xdfff if self.high_surrogate.is_some() => {
                let high = self.high_surrogate.take().expect("pending high surrogate");
                // UTF-16 splits twenty payload bits into two ten-bit halves.
                let value =
                    0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00);
                self.emit_char(char::from_u32(value).expect("paired UTF-16"));
            }
            0xd800..=0xdbff => {
                self.flush_high_surrogate();
                self.high_surrogate = Some(unit);
            }
            0xdc00..=0xdfff => {
                self.report_at(V::UnpairedSurrogate, 0);
                self.emit_char('\u{fffd}');
            }
            _ => self.emit_char(char::from_u32(u32::from(unit)).expect("non-surrogate UTF-16")),
        }
    }

    fn emit_break(&mut self, content: Content<'_>) {
        self.flush_bytes();
        if self.visible() {
            self.deliver(content);
        } else if self.current.destination == Destination::HtmlTag {
            // Both \par and \line in an HTMLTAG destination become CRLF.
            self.deliver(Content::Text("\r\n"));
        }
    }

    fn process_text(&mut self, bytes: &[u8], replace_placeholder: bool) {
        // RTF is 7-bit. Report only the first raw high byte in a text run,
        // even when the tokenizer divides that run across many buffers.
        if !self.reported_eight_bit
            && let Some(index) = bytes.iter().position(|&byte| byte >= 0x80)
        {
            self.reported_eight_bit = true;
            self.report_at(V::UnescapedEightBitCharacter, index);
        }
        if self.current.destination == Destination::Upr {
            return;
        }
        let skip = if self.unicode_skip_remaining > 0 {
            let skip = self.unicode_skip_remaining.min(bytes.len());
            self.unicode_skip_remaining -= skip;
            skip
        } else {
            usize::from(replace_placeholder && bytes.first() == Some(&b' '))
        };
        if skip == bytes.len() {
            return;
        }
        if matches!(
            self.current.destination,
            Destination::FontTable | Destination::ColorTable
        ) {
            // No need to decode raw font names or color-table text, but preceding
            // hex escapes must be flushed before these entry terminators.
            self.flush_bytes();
            for (index, &byte) in bytes.iter().enumerate().skip(skip) {
                if byte == b';' {
                    if self.current.destination == Destination::FontTable {
                        self.commit_font(index);
                    } else {
                        self.commit_color(index);
                    }
                }
            }
            return;
        }
        for &byte in &bytes[skip..] {
            // Real writers emit raw high bytes; decode them like \'hh.
            // Other raw C0 controls have no RTF meaning and are discarded.
            if byte >= b' ' || byte == b'\t' {
                self.append_byte(byte);
            }
        }
    }

    fn process_symbol(&mut self, symbol: u8) {
        // A control symbol is one ANSI fallback character, not two source bytes.
        if self.unicode_skip_remaining > 0 {
            self.unicode_skip_remaining -= 1;
            return;
        }
        if symbol == b'*' {
            self.flush_bytes();
            self.ignorable_destination = true;
            return;
        }
        self.ignorable_destination = false;
        if matches!(symbol, b'\\' | b'{' | b'}') {
            // These escaped literals can be DBCS trail bytes, so do not emit
            // Unicode characters before combining them with preceding bytes.
            if self.current.destination != Destination::Upr {
                self.append_byte(symbol);
            }
            return;
        }
        self.flush_bytes();
        if self.current.destination == Destination::Upr {
            return;
        }
        match symbol {
            b'~' => self.emit_char('\u{a0}'),
            b'_' => {
                // Encapsulating writers use \_ for "&shy;" inside HTMLTAG
                // destinations, rather than the usual nonbreaking hyphen.
                self.emit_char(if self.current.destination == Destination::HtmlTag {
                    '\u{ad}'
                } else {
                    '\u{2011}'
                });
            }
            // Optional hyphens are not rendered; escaped CR/LF are paragraphs.
            b'\n' => self.emit_break(Content::Paragraph),
            b'\t' => self.emit_char('\t'),
            _ => {}
        }
    }

    fn process_word(&mut self, word: RtfControlWord) {
        let ignorable = std::mem::take(&mut self.ignorable_destination);
        let id = word.id();
        let parameter = word.parameter().unwrap_or(0);
        // A bare toggle enables a property; only an explicit zero disables it.
        let toggle = word.parameter().is_none() || parameter != 0;
        if self.unicode_skip_remaining > 0 {
            self.unicode_skip_remaining -= 1;
            return;
        }
        if self.current.destination == Destination::FontTable {
            match id {
                K::F => {
                    self.commit_font(0);
                    self.pending_font = Some(PendingFont {
                        number: parameter,
                        charset: -1,
                        code_page: 0,
                        depth: self.depth,
                    });
                }
                K::Fcharset => {
                    if let Some(font) = &mut self.pending_font {
                        font.charset = parameter
                    }
                }
                K::Cpg => {
                    if let Some(font) = &mut self.pending_font {
                        font.code_page = parameter
                    }
                }
                K::SkipDestination => self.skip_current_group(),
                _ if ignorable => self.skip_current_group(),
                _ => {}
            }
            return;
        }
        if self.current.destination == Destination::ColorTable {
            match id {
                K::Red | K::Green | K::Blue if self.options.max_color_table_entries.is_some() => {
                    let component = match id {
                        K::Red => 0,
                        K::Green => 1,
                        _ => 2,
                    };
                    // Color components are bytes; malformed values are clamped
                    // before packing so they cannot spill into adjacent channels.
                    self.pending_color.get_or_insert([0; 3])[component] =
                        parameter.clamp(0, 255) as u8;
                }
                K::Red | K::Green | K::Blue => {}
                _ if ignorable => self.skip_current_group(),
                _ => {}
            }
            return;
        }

        match id {
            K::Ansi => self.document_code_page = 1252,
            K::Mac => self.document_code_page = 10000,
            K::Pc => self.document_code_page = 437,
            K::Pca => self.document_code_page = 850,
            K::Ansicpg => {
                if parameter > 0 {
                    self.document_code_page = parameter
                }
            }
            K::Deff => {
                self.default_font = parameter;
                self.current.font = parameter;
            }
            K::Fromhtml => {
                // Only the recognition prefix may change the document mode.
                // A bare toggle enables it, just as a nonzero parameter does.
                if self.recognition_window_open {
                    self.from_html = toggle;
                }
            }
            // HTMLRTF state is group-scoped. Control words inside it must still
            // update properties (notably fonts); only their output is suppressed
            // during extraction. Plain-text and HTML rendering still display it.
            K::Htmlrtf => self.current.set_flag(HTML_RTF, toggle),
            K::Htmltag => {
                // Ignore HTMLTagParameter and copy the destination's content.
                // RTF-rendering modes skip it like any unknown \* destination.
                if self.extracting_html() {
                    self.current.destination = Destination::HtmlTag;
                } else {
                    self.skip_current_group();
                }
            }
            K::SkipDestination => self.skip_current_group(),
            K::Fonttbl => {
                self.current.destination = Destination::FontTable;
                self.pending_font = None;
            }
            K::Colortbl => {
                self.current.destination = Destination::ColorTable;
                self.pending_color = None;
            }
            // Unsupported fields still display their results. Text conversion
            // does not collect instructions or alter link identity at all.
            K::Field => self.field_hyperlink = None,
            K::Fldinst => {
                self.current.destination = Destination::FieldInstruction;
                self.field_instruction.clear();
                self.reported_field_limit = false;
            }
            K::Fldrslt => {
                self.current.destination = Destination::Normal;
                self.current.hyperlink = self.field_hyperlink.clone();
            }
            K::Upr => self.current.destination = Destination::Upr,
            K::Ud => {
                if self.current.destination == Destination::Upr {
                    self.current.destination = Destination::Normal;
                }
            }
            // Do not skip this destination: \nestrow lives inside it, and
            // some writers also put visible nested-cell content there.
            K::Nesttableprops => {}
            K::F => {
                if !self.reported_font_limit && !self.fonts.contains_key(&parameter) {
                    self.report_at(V::UndefinedFont, 0);
                }
                self.current.font = parameter;
            }
            K::Plain => {
                self.current.flags &= !CHARACTER_FORMATTING;
                self.current.vertical_alignment = VerticalAlignment::Baseline;
                self.current.font_size = 24;
                self.current.foreground = 0;
                self.current.background = 0;
                self.current.font = self.default_font;
            }
            K::B => self.current.set_flag(BOLD, toggle),
            K::I => self.current.set_flag(ITALIC, toggle),
            K::Ul => self.current.set_flag(UNDERLINE, toggle),
            K::Ulnone => self.current.set_flag(UNDERLINE, false),
            K::Strike => self.current.set_flag(STRIKE, toggle),
            K::V => self.current.set_flag(HIDDEN, toggle),
            K::Super => self.current.vertical_alignment = VerticalAlignment::Superscript,
            K::Sub => self.current.vertical_alignment = VerticalAlignment::Subscript,
            K::Nosupersub => self.current.vertical_alignment = VerticalAlignment::Baseline,
            K::Fs => {
                // \fs without a parameter restores 12pt. Clamp explicit values
                // to Word's 1638pt maximum, as the shared source interpreter does.
                self.current.font_size = if word.parameter().is_some() {
                    if !(1..=3276).contains(&parameter) {
                        self.report_at(V::InvalidFontSize, 0)
                    }
                    parameter.clamp(0, 3276)
                } else {
                    24
                };
            }
            K::Cf | K::Cb => {
                if self.options.max_color_table_entries.is_some()
                    && parameter != 0
                    && (parameter < 0 || parameter as usize >= self.colors.len())
                    && !self.reported_color_limit
                {
                    self.report_at(V::UndefinedColor, 0);
                }
                if id == K::Cf {
                    self.current.foreground = parameter.max(0);
                } else {
                    self.current.background = parameter.max(0);
                }
            }
            K::U => {
                if self.current.destination != Destination::Upr {
                    // Signed UTF-16 units above 32767 are written as negatives.
                    let value = if parameter < 0 {
                        parameter + 65536
                    } else {
                        parameter
                    };
                    if let Ok(unit) = u16::try_from(value) {
                        self.emit_unicode(unit);
                    } else {
                        if !word.is_parameter_invalid() {
                            self.report_at(V::InvalidParameter, 0)
                        }
                        self.emit_char('\u{fffd}');
                    }
                }
                self.unicode_skip_remaining = self.current.unicode_skip;
            }
            K::Uc => self.current.unicode_skip = parameter.max(0) as usize,
            K::Par => self.emit_break(Content::Paragraph),
            K::Line => self.emit_break(Content::LineBreak),
            K::Tab => self.emit_char('\t'),
            K::Pard => {
                self.current.alignment = Alignment::Left;
                self.current.set_flag(IN_TABLE, false);
            }
            K::Ql => self.current.alignment = Alignment::Left,
            K::Qc => self.current.alignment = Alignment::Center,
            K::Qr => self.current.alignment = Alignment::Right,
            K::Qj => self.current.alignment = Alignment::Justify,
            K::Intbl => self.current.set_flag(IN_TABLE, true),
            K::Cell if self.visible() => {
                self.deliver(Content::Cell);
            }
            K::Row if self.visible() => {
                self.deliver(Content::Row);
            }
            K::Cell | K::Row => {}
            K::Objattph => {
                if self.visible() {
                    let index = self.placeholder_index;
                    self.placeholder_index = self.placeholder_index.saturating_add(1);
                    self.deliver(Content::ObjectPlaceholder(index));
                    // Replace only an immediately following literal/hex space;
                    // malformed writers may omit that character entirely.
                    self.object_placeholder_pending = true;
                }
            }
            _ => {
                if let Some(c) = special_character(id) {
                    self.emit_char(c);
                } else if ignorable {
                    // Unknown words are ignored, but \* marks their entire
                    // group as an ignorable destination.
                    self.skip_current_group();
                }
            }
        }
    }
}

fn special_character(id: K) -> Option<char> {
    Some(match id {
        K::Emdash => '\u{2014}',
        K::Endash => '\u{2013}',
        K::Bullet => '\u{2022}',
        K::Lquote => '\u{2018}',
        K::Rquote => '\u{2019}',
        K::Ldblquote => '\u{201c}',
        K::Rdblquote => '\u{201d}',
        K::Emspace => '\u{2003}',
        K::Enspace => '\u{2002}',
        K::Qmspace => '\u{2005}',
        K::Zwj => '\u{200d}',
        K::Zwnj => '\u{200c}',
        K::Ltrmark => '\u{200e}',
        K::Rtlmark => '\u{200f}',
        _ => return None,
    })
}

fn font_code_page(charset: i32) -> i32 {
    match charset {
        77 => 10000,
        128 => 932,
        129 => 949,
        130 => 1361,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        254 => 437,
        255 => 850,
        // ANSI, default, symbol, and unknown charsets use the document page.
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Discard;

    impl ContentHandler for Discard {
        fn content(&mut self, _: &GroupState, _: &[Option<u32>], _: Content<'_>) {}
    }

    fn options() -> InterpreterOptions {
        InterpreterOptions {
            max_group_depth: DEFAULT_LIMIT,
            max_font_table_entries: DEFAULT_LIMIT,
            max_color_table_entries: None,
            collect_hyperlinks: false,
            extract_html: false,
            max_issues_per_violation: 0,
        }
    }

    #[test]
    fn recognition_notifies_once_before_content_or_at_eof() {
        #[derive(Default)]
        struct Recording {
            began: Option<bool>,
            text: String,
        }

        impl ContentHandler for Recording {
            fn begin(&mut self, from_html: bool) {
                assert_eq!(self.began, None);
                self.began = Some(from_html);
            }

            fn content(&mut self, _: &GroupState, _: &[Option<u32>], content: Content<'_>) {
                assert!(self.began.is_some());
                if let Content::Text(text) = content {
                    self.text.push_str(text);
                }
            }
        }

        let mut options = options();
        options.extract_html = true;
        for (input, from_html, text) in [
            ("", false, ""),
            ("{\\rtf1\\ansi", false, ""),
            ("{\\rtf1\\fromhtml1", true, ""),
            ("{\\rtf1\\fromhtml1 x}", true, "x"),
            ("{\\rtf1\\fromhtml1\\tab\\fromhtml0 x}", true, "\tx"),
            ("{\\rtf1\\tab\\fromhtml1 x}", false, "\tx"),
        ] {
            let handler = interpret(
                input.as_bytes(),
                options,
                None::<fn(RtfIssue)>,
                Recording::default(),
            )
            .unwrap();
            assert_eq!(handler.began, Some(from_html));
            assert_eq!(handler.text, text);
        }
    }

    #[test]
    fn html_extraction_retains_bounded_interpreter_state() {
        let mut options = options();
        options.extract_html = true;
        options.max_group_depth = NonZeroUsize::new(2).unwrap();
        options.max_font_table_entries = NonZeroUsize::new(1).unwrap();
        let input = format!(
            "{{\\rtf1\\fromhtml1{{\\*\\htmltag {}x{}}}}}",
            "{".repeat(100_000),
            "}".repeat(100_000),
        );
        let interpreter = run(&options, input.as_bytes());
        assert!(interpreter.from_html);
        assert!(!interpreter.reported_group_limit);
        assert!(interpreter.stack.is_empty());
        assert!(interpreter.recognition_tokens <= 11);

        // Neither HTML extraction nor plain-text rendering needs to retain
        // field instructions or color-table entries.
        let input = format!(
            "{{\\rtf1\\fromhtml1{{\\colortbl;\\red1\\green2\\blue3;}}{{\\*\\fldinst {}}}{{\\*\\htmltag {}}}}}",
            "x".repeat(10_000),
            "y".repeat(10_000),
        );
        let interpreter = run(&options, input.as_bytes());
        assert!(interpreter.field_instruction.is_empty());
        assert!(interpreter.colors.is_empty());
    }

    type TestInterpreter = Interpreter<fn(RtfIssue), Discard>;

    fn run(options: &InterpreterOptions, input: &[u8]) -> TestInterpreter {
        let mut interpreter = Interpreter::new(*options, None, Discard);
        let mut validator = RtfValidator::new(input);
        while let Some(token) = validator.read_token().unwrap() {
            interpreter.process(token);
            assert!(interpreter.stack.len() <= options.max_group_depth.get());
            assert!(interpreter.stack.capacity() <= options.max_group_depth.get());
            assert!(interpreter.fonts.len() <= options.max_font_table_entries.get());
            assert!(interpreter.code_pages.len() <= MAX_CODE_PAGES);
            assert!(interpreter.byte_count <= BYTE_BUFFER_SIZE);
            let colors = options.max_color_table_entries.map_or(0, NonZeroUsize::get);
            assert!(interpreter.colors.len() <= colors);
            assert!(interpreter.colors.capacity() <= colors);
            assert!(interpreter.field_instruction.len() <= MAX_FIELD_INSTRUCTION_LENGTH);
            assert!(interpreter.field_instruction.capacity() <= MAX_FIELD_INSTRUCTION_LENGTH);
        }
        interpreter.position = validator.source_position();
        interpreter.flush_bytes();
        interpreter.flush_high_surrogate();
        interpreter
    }

    #[test]
    fn folded_and_discarded_groups_retain_bounded_state() {
        let mut options = options();
        options.max_group_depth = NonZeroUsize::new(1).unwrap();
        let input = format!("{{\\rtf1 {}", "{".repeat(100_000));
        let interpreter = run(&options, input.as_bytes());
        assert_eq!(interpreter.stack.len(), 1);
        assert_eq!(interpreter.stack[0].count, 100_001);
        assert_eq!(interpreter.depth, 100_001);
        assert_eq!(interpreter.skip_depth, 0);

        let input = format!("{{\\rtf1 {}", "{\\b{\\b0 ".repeat(1000));
        for limit in [1, 2, 8, 17, 31] {
            options.max_group_depth = NonZeroUsize::new(limit).unwrap();
            let interpreter = run(&options, input.as_bytes());
            assert_eq!(interpreter.stack.len(), limit);
            assert_eq!(interpreter.depth + interpreter.skip_depth, 2001);
            assert!(!interpreter.skip_pops_group);
        }
        let input = format!("{input}{}", "}".repeat(2001));
        let interpreter = run(&options, input.as_bytes());
        assert_eq!(interpreter.depth, 0);
        assert_eq!(interpreter.skip_depth, 0);
        assert!(interpreter.stack.is_empty());
    }

    #[test]
    fn font_and_code_page_storage_stays_bounded() {
        let mut options = options();
        options.max_font_table_entries = NonZeroUsize::new(3).unwrap();
        let mut input = String::from("{\\rtf1{\\fonttbl");
        for i in 0..1000 {
            input.push_str(&format!("{{\\f{i} name;}}"));
        }
        input.push('}');
        for i in 0..100 {
            input.push_str(&format!("\\ansicpg{}\\'e9", 70000 + i));
        }
        input.push('}');
        let interpreter = run(&options, input.as_bytes());
        assert_eq!(interpreter.fonts.len(), 3);
        assert_eq!(interpreter.code_pages.len(), MAX_CODE_PAGES);
        assert!(interpreter.pending_font.is_none());
        assert!(interpreter.decoder.is_none());
        assert_eq!(interpreter.byte_count, 0);
        assert!(interpreter.decoded.is_empty());
    }

    #[test]
    fn color_and_nested_link_state_remain_bounded() {
        let mut options = options();
        options.max_color_table_entries = NonZeroUsize::new(2);
        options.collect_hyperlinks = true;
        options.max_group_depth = NonZeroUsize::new(17).unwrap();
        let mut input = format!("{{\\rtf1{{\\colortbl;{}}}", "\\red1;".repeat(10_000));
        for index in 0..1000 {
            input.push_str(&format!(
                "{{\\field{{\\*\\fldinst HYPERLINK \"https://{index}/\"}}{{\\fldrslt "
            ));
        }
        input.push('x');
        input.push_str(&"}}".repeat(1000));
        input.push('}');
        let interpreter = run(&options, input.as_bytes());
        assert_eq!(interpreter.colors, [None, Some(0x010000)]);
        assert!(interpreter.reported_color_limit);
        assert!(interpreter.reported_group_limit);
        assert_eq!(interpreter.depth, 0);
        assert_eq!(interpreter.skip_depth, 0);
        assert!(interpreter.stack.is_empty());
        assert!(interpreter.current.hyperlink.is_none());
    }

    #[test]
    fn link_identity_not_target_content_controls_state_folding() {
        let a = Hyperlink(Rc::from("https://example.com/"));
        let b = a.clone();
        let c = Hyperlink(Rc::from("https://example.com/"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        let mut left = GroupState::default();
        let mut right = left.clone();
        left.hyperlink = Some(a);
        right.hyperlink = Some(b);
        assert_eq!(left, right);
        right.hyperlink = Some(c);
        assert_ne!(left, right);
    }

    #[test]
    fn font_charset_mapping_matches_the_source_table() {
        for (charset, page) in [
            (77, 10000),
            (128, 932),
            (129, 949),
            (130, 1361),
            (134, 936),
            (136, 950),
            (161, 1253),
            (162, 1254),
            (163, 1258),
            (177, 1255),
            (178, 1256),
            (186, 1257),
            (204, 1251),
            (222, 874),
            (238, 1250),
            (254, 437),
            (255, 850),
            (0, 0),
            (1, 0),
            (2, 0),
            (-1, 0),
            (999, 0),
        ] {
            assert_eq!(font_code_page(charset), page);
        }
    }
}

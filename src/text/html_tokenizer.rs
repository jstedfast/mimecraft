// Adapted from MimeKit's Text/HtmlTokenizer.cs and Text/HtmlTokenizerState.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::VecDeque, error::Error, fmt, mem, str::Utf8Error};

use super::{
    HtmlAttribute, HtmlAttributeQuote, HtmlCommentToken, HtmlContentMode, HtmlDataToken,
    HtmlDocTypeToken, HtmlEntityDecoder, HtmlNamespace, HtmlTagToken, HtmlToken,
    HtmlTreeConstruction, InvalidHtmlElementDepth,
};

/// Options copied when constructing an [`HtmlTokenizer`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HtmlTokenizerOptions {
    /// Decode references in data and RCDATA. Attribute references always decode.
    pub decode_character_references: bool,
    /// Discard incomplete tags at EOF instead of returning encoded literal text.
    pub ignore_truncated_tags: bool,
    /// Treat HTML noscript contents as raw text.
    pub scripting_enabled: bool,
    /// Positive limit on distinct structural stack entries; identical runs fold.
    pub max_element_depth: usize,
}

impl Default for HtmlTokenizerOptions {
    fn default() -> Self {
        Self {
            decode_character_references: true,
            ignore_truncated_tags: false,
            scripting_enabled: true,
            max_element_depth: 4096,
        }
    }
}

/// Position of the next unconsumed Unicode scalar in the UTF-8 source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HtmlSourcePosition {
    /// Zero-based UTF-8 byte offset, unlike MimeKit's UTF-16 offset.
    pub offset: u64,
    /// One-based line number; only LF advances the line.
    pub line: u64,
    /// One-based Unicode-scalar column. CR is an ordinary column.
    pub column: u64,
}

/// Outcome of advancing the sans-I/O tokenizer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HtmlTokenizerResult {
    /// A complete owned token.
    Token(HtmlToken),
    /// More bytes are needed. This is not EOF and does not finalize partial input.
    NeedInput,
    /// Input was explicitly finished and all tokens have been returned.
    EndOfInput,
}

/// Rejected input or terminal UTF-8 decoding failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlTokenizerError {
    /// Bytes were supplied after [`HtmlTokenizer::finish`].
    InputFinished,
    /// Invalid UTF-8, or an incomplete scalar at explicit EOF.
    InvalidUtf8(Utf8Error),
}

impl fmt::Display for HtmlTokenizerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputFinished => f.write_str("HTML input has already been finished"),
            Self::InvalidUtf8(error) => write!(f, "invalid UTF-8 in HTML input: {error}"),
        }
    }
}

impl Error for HtmlTokenizerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidUtf8(error) => Some(error),
            _ => None,
        }
    }
}

enum ScanError {
    NeedInput,
    InvalidUtf8(Utf8Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EndContext {
    RcData,
    RawText,
    Script,
    EscapedScript,
}

impl EndContext {
    fn state(self) -> State {
        match self {
            Self::RcData => State::Text(HtmlContentMode::RcData),
            Self::RawText => State::Text(HtmlContentMode::RawText),
            Self::Script => State::Text(HtmlContentMode::ScriptData),
            Self::EscapedScript => State::ScriptEscaped(0),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReferenceContext {
    Data,
    RcData,
    Attribute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Text(HtmlContentMode),
    DataLess,
    Less(HtmlContentMode),
    TagOpen,
    EndTagOpen,
    TagName,
    EndOpen(EndContext),
    EndName(EndContext),
    Reference(ReferenceContext),
    ScriptEscapeStart(bool),
    ScriptEscaped(u8),
    ScriptEscapedLess,
    ScriptDoubleWord(bool),
    ScriptDoubleEscaped(u8),
    ScriptDoubleLess,
    BeforeAttribute,
    AttributeName,
    AfterAttribute,
    BeforeValue,
    Value,
    AfterValue,
    SelfClosing,
    Declaration,
    BogusComment,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    DocType,
    BeforeDocName,
    DocName,
    AfterDocName,
    BeforeDocIdentifier(bool),
    DocIdentifier(bool),
    AfterPublicIdentifier,
    AfterSystemIdentifier,
    BogusDocType,
    CData,
    Eof,
}

/// Sans-I/O HTML tokenizer over incrementally supplied UTF-8 bytes.
///
/// Ports MimeKit's data, RCDATA, RAWTEXT, script escaped/double-escaped,
/// attribute, comment, DOCTYPE, and foreign CDATA states. Structural processing
/// selects content modes, rather than assuming every `style` is HTML raw text.
/// Original tag/attribute spelling and duplicate attributes are preserved;
/// malformed attribute names are available for inspection, not silently dropped.
///
/// Input must already be UTF-8: there is no charset sniffing, BOM removal, or
/// line-ending normalization. Use decoded text or a charset-decoding reader for
/// other encodings. A BOM in UTF-8 input is ordinary character data.
/// Owned payloads can grow with a token; the element-depth limit is not a token
/// length or total-memory limit. No DOM, sanitization, or compliance logger is
/// supplied. The structural controller has its documented partial-tree limits.
///
/// Supply bytes with [`Self::push`], drain tokens with [`Self::next_token`],
/// and explicitly signal EOF with [`Self::finish`]. An empty input chunk is
/// not EOF. Partial UTF-8 scalars, references, and markup survive `NeedInput`
/// without finalizing or changing token boundaries. This engine performs no
/// reads, awaits, or callbacks and can be driven by synchronous or async code.
/// [`super::HtmlTokenizerReader`] supplies the synchronous `BufRead` adapter.
///
/// Invalid UTF-8 is terminal, not replacement text. Malformed *HTML* instead
/// follows MimeKit's recovery rules. Supplied bytes are copied into a queue;
/// feed bounded chunks and drain before supplying more to bound queued input.
/// Reference decoding is fixed at construction so suspending a partial token
/// cannot change its interpretation or serialization policy.
///
/// ```
/// use mimecraft::text::{HtmlTokenizer, HtmlToken, HtmlTokenizerResult};
///
/// let mut tokenizer = HtmlTokenizer::new();
/// tokenizer.push(b"<p>one &am")?;
/// assert!(matches!(tokenizer.next_token()?, HtmlTokenizerResult::Token(HtmlToken::Tag(_))));
/// assert_eq!(tokenizer.next_token()?, HtmlTokenizerResult::NeedInput);
/// tokenizer.push(b"p; two</p>")?;
/// tokenizer.finish();
/// let HtmlTokenizerResult::Token(HtmlToken::Data(data)) = tokenizer.next_token()? else { panic!() };
/// assert_eq!(data.data(), "one & two");
/// # Ok::<(), mimecraft::text::HtmlTokenizerError>(())
/// ```
#[derive(Debug)]
pub struct HtmlTokenizer {
    input: VecDeque<u8>,
    buffer: String,
    buffer_position: usize,
    state: State,
    tree: HtmlTreeConstruction,
    decode: bool,
    ignore_truncated: bool,
    data: String,
    name: String,
    tag_name: String,
    attributes: Vec<HtmlAttribute>,
    end_tag: bool,
    quote: Option<char>,
    active_tag: String,
    doctype: Option<HtmlDocTypeToken>,
    bang: bool,
    entity: HtmlEntityDecoder,
    cdata_brackets: usize,
    lookahead: Option<char>,
    utf8: [u8; 4],
    utf8_length: usize,
    input_eof: bool,
    invalid_utf8: Option<Utf8Error>,
    position: HtmlSourcePosition,
}

impl Default for HtmlTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlTokenizer {
    /// Creates a tokenizer using the default options.
    pub fn new() -> Self {
        Self::with_options(HtmlTokenizerOptions::default()).expect("positive default depth")
    }

    /// Creates a tokenizer and copies its options.
    ///
    /// # Errors
    /// Returns [`InvalidHtmlElementDepth`] if the maximum depth is zero.
    pub fn with_options(options: HtmlTokenizerOptions) -> Result<Self, InvalidHtmlElementDepth> {
        let mut tree = HtmlTreeConstruction::with_max_element_depth(options.max_element_depth)?;
        tree.set_scripting_enabled(options.scripting_enabled);
        Ok(Self {
            input: VecDeque::new(),
            buffer: String::new(),
            buffer_position: 0,
            tree,
            state: State::Text(HtmlContentMode::Data),
            decode: options.decode_character_references,
            ignore_truncated: options.ignore_truncated_tags,
            data: String::new(),
            name: String::new(),
            tag_name: String::new(),
            attributes: Vec::new(),
            end_tag: false,
            quote: None,
            active_tag: String::new(),
            doctype: None,
            bang: false,
            entity: HtmlEntityDecoder::new(),
            cdata_brackets: 0,
            lookahead: None,
            utf8: [0; 4],
            utf8_length: 0,
            input_eof: false,
            invalid_utf8: None,
            position: HtmlSourcePosition {
                offset: 0,
                line: 1,
                column: 1,
            },
        })
    }

    /// Copies another input chunk. Empty chunks do not signal EOF.
    ///
    /// # Errors
    /// Rejects bytes after explicit EOF or a terminal decoding error. No bytes
    /// are accepted on error. UTF-8 validation occurs while advancing tokens.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), HtmlTokenizerError> {
        if let Some(error) = self.invalid_utf8 {
            return Err(HtmlTokenizerError::InvalidUtf8(error));
        }
        if self.input_eof {
            return Err(HtmlTokenizerError::InputFinished);
        }
        self.input.extend(bytes);
        Ok(())
    }

    /// Signals that no more bytes will arrive; repeated calls are harmless.
    ///
    /// Drain remaining tokens with [`Self::next_token`]. Incomplete markup
    /// recovers only after this call, never at an ordinary chunk boundary.
    pub fn finish(&mut self) {
        self.input_eof = true;
    }

    /// Advances until a token, a need for input, or explicit EOF is reached.
    ///
    /// # Errors
    /// Invalid UTF-8 returns [`HtmlTokenizerError::InvalidUtf8`]. That error is
    /// sticky; construct a new tokenizer to process a different input.
    pub fn next_token(&mut self) -> Result<HtmlTokenizerResult, HtmlTokenizerError> {
        match self.advance() {
            Ok(Some(token)) => Ok(HtmlTokenizerResult::Token(token)),
            Ok(None) => Ok(HtmlTokenizerResult::EndOfInput),
            Err(ScanError::NeedInput) => Ok(HtmlTokenizerResult::NeedInput),
            Err(ScanError::InvalidUtf8(error)) => Err(HtmlTokenizerError::InvalidUtf8(error)),
        }
    }

    /// Returns the next logical source position, excluding buffered lookahead.
    pub fn position(&self) -> HtmlSourcePosition {
        self.position
    }

    /// Returns whether EOF has been reached and all tokens have been emitted.
    pub fn is_eof(&self) -> bool {
        self.state == State::Eof && self.data.is_empty()
    }

    /// Borrows structural state, including the last tag result and depth status.
    pub fn tree_construction(&self) -> &HtmlTreeConstruction {
        &self.tree
    }

    /// Enables optional implicit-closure collection for subsequent tokens.
    pub fn set_track_closed_elements(&mut self, enabled: bool) {
        self.tree.set_track_closed_elements(enabled);
    }

    /// Clears collected implicit closures without changing structural state.
    ///
    /// Consumers should clear these after processing a token: character data
    /// can append closures, while tags start a new collection.
    pub fn clear_closed_elements(&mut self) {
        self.tree.clear_closed_elements();
    }

    /// Returns the declared html namespace, not the current foreign namespace.
    pub fn html_namespace(&self) -> HtmlNamespace {
        self.tree.html_namespace()
    }

    /// Returns whether data/RCDATA character references are decoded.
    pub fn decode_character_references(&self) -> bool {
        self.decode
    }

    /// Returns whether EOF-truncated tags are discarded.
    pub fn ignore_truncated_tags(&self) -> bool {
        self.ignore_truncated
    }

    /// Changes the EOF-truncated-tag policy.
    pub fn set_ignore_truncated_tags(&mut self, enabled: bool) {
        self.ignore_truncated = enabled;
    }

    /// Changes noscript handling for subsequent tags.
    pub fn set_scripting_enabled(&mut self, enabled: bool) {
        self.tree.set_scripting_enabled(enabled);
    }

    /// Changes the distinct-entry depth limit without clearing prior overflow.
    ///
    /// # Errors
    /// Returns [`InvalidHtmlElementDepth`] for zero without changing the limit.
    pub fn set_max_element_depth(&mut self, maximum: usize) -> Result<(), InvalidHtmlElementDepth> {
        self.tree.set_max_element_depth(maximum)
    }

    fn fill_buffer(&mut self) {
        if self.buffer_position < self.buffer.len()
            || self.lookahead.is_some()
            || self.utf8_length != 0
            || self.invalid_utf8.is_some()
        {
            return;
        }
        // Validate each contiguous input span once, not every name/value run.
        // Keep malformed or incomplete bytes queued: their error is observable
        // only when the scalar decoder reaches them, after any preceding tokens.
        let bytes = self.input.as_slices().0;
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                std::str::from_utf8(&bytes[..error.valid_up_to()]).expect("valid UTF-8 prefix")
            }
        };
        self.buffer.clear();
        self.buffer.push_str(text);
        self.buffer_position = 0;
        self.input.drain(..text.len());
    }

    fn peek(&mut self) -> Result<Option<char>, ScanError> {
        if let Some(error) = self.invalid_utf8 {
            return Err(ScanError::InvalidUtf8(error));
        }
        if self.lookahead.is_some() {
            return Ok(self.lookahead);
        }
        if self.buffer_position == self.buffer.len() {
            self.fill_buffer();
        }
        if let Some(c) = self.buffer[self.buffer_position..].chars().next() {
            self.buffer_position += c.len_utf8();
            self.lookahead = Some(c);
            return Ok(self.lookahead);
        }
        loop {
            let end = self.input_eof && self.input.is_empty();
            if self.utf8_length > 0 {
                match std::str::from_utf8(&self.utf8[..self.utf8_length]) {
                    Ok(text) => {
                        self.lookahead = text.chars().next();
                        self.utf8_length = 0;
                        return Ok(self.lookahead);
                    }
                    // Only a valid, incomplete prefix can wait for more input.
                    // A known bad continuation must fail without waiting for EOF.
                    Err(error) if error.error_len().is_none() && !end => {}
                    Err(error) => {
                        self.invalid_utf8 = Some(error);
                        return Err(ScanError::InvalidUtf8(error));
                    }
                }
            }
            if end {
                return Ok(None);
            }
            // Keep partial scalars across arbitrarily short chunks. Temporary
            // exhaustion is distinct from explicit EOF, including within UTF-8.
            if let Some(byte) = self.input.pop_front() {
                self.utf8[self.utf8_length] = byte;
                self.utf8_length += 1;
            } else {
                return Err(ScanError::NeedInput);
            }
        }
    }

    fn consume(&mut self, c: char) {
        debug_assert_eq!(self.lookahead, Some(c));
        self.lookahead = None;
        self.position.offset += c.len_utf8() as u64;
        if c == '\n' {
            self.position.line += 1;
            self.position.column = 1;
        } else {
            self.position.column += 1;
        }
    }

    fn read_run(&mut self) {
        use HtmlContentMode as Mode;
        use State::*;

        // Like MimeKit's TryReadDataUntil/TryReadNameUntil, copy ordinary runs
        // and leave the special character to the lexical state machine. Unlike
        // TextReader, a byte chunk can end inside UTF-8; fill_buffer leaves
        // incomplete scalars to the resumable decoder.
        if self.lookahead.is_some() || self.utf8_length != 0 || self.invalid_utf8.is_some() {
            return;
        }
        if self.buffer_position == self.buffer.len() {
            self.fill_buffer();
        }
        let buffered = &self.buffer[self.buffer_position..];
        let bytes = buffered.as_bytes();
        if bytes.is_empty() {
            return;
        }
        let (end, data, name) = match self.state {
            Text(Mode::Data) => (memchr::memchr2(b'<', b'&', bytes), true, false),
            Text(Mode::RcData) => (memchr::memchr3(b'<', b'&', 0, bytes), true, false),
            Text(Mode::RawText | Mode::ScriptData) => {
                (memchr::memchr2(b'<', 0, bytes), true, false)
            }
            Text(Mode::PlainText) => (memchr::memchr(0, bytes), true, false),
            Value if self.quote.is_some() => (
                memchr::memchr3(self.quote.unwrap() as u8, b'&', 0, bytes),
                true,
                true,
            ),
            Value => (
                bytes
                    .iter()
                    .position(|&b| byte_space(b) || matches!(b, b'&' | b'>' | 0)),
                true,
                true,
            ),
            TagName => (
                bytes
                    .iter()
                    .position(|&b| byte_space(b) || matches!(b, b'/' | b'>' | 0)),
                true,
                true,
            ),
            AttributeName => (
                bytes
                    .iter()
                    .position(|&b| byte_space(b) || matches!(b, b'/' | b'=' | b'>' | 0)),
                true,
                true,
            ),
            EndName(_) => (
                bytes.iter().position(|b| !b.is_ascii_alphabetic()),
                true,
                true,
            ),
            BeforeAttribute | AfterAttribute | BeforeValue => {
                (bytes.iter().position(|&b| !byte_space(b)), true, false)
            }
            Comment => (memchr::memchr2(b'-', 0, bytes), false, true),
            BogusComment => (memchr::memchr2(b'>', 0, bytes), true, false),
            DocIdentifier(_) => (
                memchr::memchr3(self.quote.unwrap() as u8, b'>', 0, bytes),
                false,
                true,
            ),
            CData if self.cdata_brackets == 0 => (memchr::memchr(b']', bytes), true, false),
            ScriptEscaped(0) | ScriptDoubleEscaped(0) => {
                (memchr::memchr3(b'-', b'<', 0, bytes), true, false)
            }
            _ => return,
        };
        let end = end.unwrap_or(bytes.len());
        if end == 0 {
            return;
        }
        let text = &buffered[..end];
        let length = text.len();
        if length == 0 {
            return;
        }
        if data {
            self.data.push_str(text);
        }
        if name {
            self.name.push_str(text);
        }
        if self.state == Text(Mode::Data) && self.tree.tracks_character_data() {
            self.tree.process_character_data(text);
        }
        // Update once per run, retaining scalar columns rather than UTF-8 bytes.
        update_position(&mut self.position, text);
        self.buffer_position += length;
    }

    fn data_token(&mut self, encode: bool, data_state: bool) -> Option<HtmlToken> {
        if self.data.is_empty() {
            return None;
        }
        Some(HtmlToken::Data(HtmlDataToken::from_tokenizer(
            &mut self.data,
            encode,
            data_state,
        )))
    }

    fn read_tag(&mut self) -> Option<HtmlToken> {
        if self.lookahead.is_some() || self.utf8_length != 0 || self.invalid_utf8.is_some() {
            return None;
        }
        if self.buffer_position == self.buffer.len() {
            self.fill_buffer();
        }
        let text = &self.buffer[self.buffer_position..];
        let end = memchr::memchr3(b'>', b'&', 0, text.as_bytes())?;
        if text.as_bytes()[end] != b'>' {
            return None;
        }
        let text = &text[..=end];
        // Complete ordinary tags can borrow name/value spans while building the
        // token, without copying raw markup into the truncated-tag buffer.
        // References, NUL, quoted '>', and incomplete/malformed closing syntax
        // fall back without consuming anything or changing structural state.
        debug_assert!(self.attributes.is_empty());
        let Some((name_end, empty)) = scan_tag_attributes(text, &mut self.attributes) else {
            self.attributes.clear();
            return None;
        };
        self.name.push_str(&text[..name_end]);
        mem::swap(&mut self.name, &mut self.tag_name);
        update_position(&mut self.position, text);
        self.buffer_position += text.len();
        Some(self.tag_token(empty))
    }

    fn script_token(&mut self) -> Option<HtmlToken> {
        if self.data.is_empty() {
            None
        } else {
            Some(HtmlToken::ScriptData(mem::take(&mut self.data)))
        }
    }

    fn text_token(&mut self, mode: HtmlContentMode) -> Option<HtmlToken> {
        match mode {
            HtmlContentMode::ScriptData => self.script_token(),
            HtmlContentMode::Data => self.data_token(self.decode, true),
            HtmlContentMode::RcData => self.data_token(self.decode, false),
            HtmlContentMode::RawText => self.data_token(false, false),
            // After depth overflow, literal text is not plaintext-element content:
            // encode it on serialization so untracked markup cannot become active.
            HtmlContentMode::PlainText => self.data_token(self.tree.depth_exceeded(), false),
        }
    }

    fn end_context_token(&mut self, context: EndContext) -> Option<HtmlToken> {
        match context {
            EndContext::RcData => self.data_token(self.decode, false),
            EndContext::RawText => self.data_token(false, false),
            _ => self.script_token(),
        }
    }

    fn tag_token(&mut self, empty: bool) -> HtmlToken {
        let attributes = mem::take(&mut self.attributes);
        let tag = HtmlTagToken::from_tokenizer(&mut self.tag_name, attributes, empty, self.end_tag);
        let result = self.tree.process_tag(&tag);
        self.state = State::Text(result.content_mode);
        if matches!(
            result.content_mode,
            HtmlContentMode::RawText | HtmlContentMode::RcData
        ) {
            self.active_tag = tag.id().as_str().into();
        }
        self.data.clear();
        self.name.clear();
        self.quote = None;
        HtmlToken::Tag(tag)
    }

    fn eof_tag(&mut self) -> Option<HtmlToken> {
        self.state = State::Eof;
        self.name.clear();
        self.tag_name.clear();
        self.attributes.clear();
        if self.ignore_truncated {
            self.data.clear();
            None
        } else {
            self.data_token(true, false)
        }
    }

    fn attribute_name(&mut self) {
        self.attributes
            .push(HtmlAttribute::from_tokenizer(&mut self.name, None));
    }

    fn attribute_value(&mut self) {
        self.attributes
            .last_mut()
            .expect("attribute before value")
            .set_tokenizer_value(&mut self.name);
    }

    fn reference_start(&mut self, context: ReferenceContext) {
        self.entity.reset();
        self.entity
            .push('&')
            .expect("reference starts with ampersand");
        self.state = State::Reference(context);
    }

    fn reference_end(&mut self, context: ReferenceContext, next: Option<char>) {
        match context {
            ReferenceContext::Attribute => {
                self.name.push_str(&self.entity.attribute_value(next));
                self.state = State::Value;
            }
            ReferenceContext::Data | ReferenceContext::RcData => {
                let value = self.entity.value();
                if context == ReferenceContext::Data {
                    self.tree.process_character_data(&value);
                }
                if self.decode {
                    self.data.push_str(&value);
                }
                self.state = State::Text(if context == ReferenceContext::Data {
                    HtmlContentMode::Data
                } else {
                    HtmlContentMode::RcData
                });
            }
        }
        self.entity.reset();
    }

    fn comment_token(&mut self, bogus: bool) -> HtmlToken {
        let contents = if bogus {
            mem::take(&mut self.data)
        } else {
            mem::take(&mut self.name)
        };
        let comment = if !bogus {
            HtmlCommentToken::new(contents)
        } else if self.bang {
            HtmlCommentToken::bang(contents)
        } else {
            HtmlCommentToken::bogus(contents)
        };
        self.bang = false;
        self.name.clear();
        self.data.clear();
        self.state = State::Text(HtmlContentMode::Data);
        HtmlToken::Comment(comment)
    }

    fn doc_token(&mut self) -> HtmlToken {
        // An incomplete PUBLIC/SYSTEM keyword must not leak into the next tag.
        self.name.clear();
        self.data.clear();
        // An abruptly terminated quoted identifier must not affect an unquoted
        // attribute reference in a subsequent tag.
        self.quote = None;
        HtmlToken::DocType(self.doctype.take().expect("DOCTYPE state"))
    }

    fn doc_name(&mut self) {
        self.doctype
            .as_mut()
            .unwrap()
            .set_name(Some(mem::take(&mut self.name)))
            .expect("tokenizer excludes DOCTYPE name delimiters");
    }

    fn doc_identifier(&mut self, public: bool) {
        let value = Some(mem::take(&mut self.name));
        let doc = self.doctype.as_mut().unwrap();
        let result = if public {
            doc.set_public_identifier(value)
        } else {
            doc.set_system_identifier(value)
        };
        result.expect("tokenizer excludes identifier delimiters");
    }

    fn advance(&mut self) -> Result<Option<HtmlToken>, ScanError> {
        use HtmlContentMode as Mode;
        use State::*;
        loop {
            if self.state == Eof {
                return Ok(None);
            }
            if self.state == TagName
                && let Some(token) = self.read_tag()
            {
                return Ok(Some(token));
            }
            self.read_run();
            // Special-character transitions consume at most one scalar. All state
            // survives suspension; reconsumption never rewinds or duplicates input.
            let next = self.peek()?;
            let Some(c) = next else {
                let state = self.state;
                let token = match state {
                    Text(mode) => self.text_token(mode),
                    DataLess => {
                        self.state = TagOpen;
                        let token = self.text_token(Mode::Data);
                        if token.is_some() {
                            return Ok(token);
                        }
                        continue;
                    }
                    TagOpen => {
                        self.data.push('<');
                        self.data_token(true, false)
                    }
                    EndTagOpen => self.data_token(true, false),
                    Less(mode) => {
                        self.data.push('<');
                        self.text_token(mode)
                    }
                    EndOpen(context) | EndName(context) => self.end_context_token(context),
                    Reference(context) => {
                        if context == ReferenceContext::Attribute {
                            self.eof_tag()
                        } else {
                            self.reference_end(context, None);
                            continue;
                        }
                    }
                    ScriptEscapeStart(_)
                    | ScriptEscaped(_)
                    | ScriptEscapedLess
                    | ScriptDoubleWord(_)
                    | ScriptDoubleEscaped(_)
                    | ScriptDoubleLess => self.script_token(),
                    Declaration => {
                        self.data = mem::take(&mut self.name);
                        self.bang = true;
                        Some(self.comment_token(true))
                    }
                    BogusComment => Some(self.comment_token(true)),
                    CommentStart | CommentStartDash | Comment | CommentEndDash | CommentEnd
                    | CommentEndBang => Some(self.comment_token(false)),
                    DocType
                    | BeforeDocName
                    | DocName
                    | AfterDocName
                    | BeforeDocIdentifier(_)
                    | DocIdentifier(_)
                    | AfterPublicIdentifier
                    | AfterSystemIdentifier
                    | BogusDocType => {
                        if state == DocName {
                            self.doc_name();
                        }
                        if let DocIdentifier(public) = state {
                            self.doc_identifier(public);
                        }
                        // Unlike other DOCTYPE states, bogus-DOCTYPE EOF does not force quirks.
                        if state != BogusDocType {
                            self.doctype.as_mut().unwrap().force_quirks_mode = true;
                        }
                        Some(self.doc_token())
                    }
                    CData => {
                        for _ in 0..self.cdata_brackets {
                            self.data.push(']');
                        }
                        self.cdata_brackets = 0;
                        self.tree.process_character_data(&self.data);
                        if self.data.is_empty() {
                            None
                        } else {
                            Some(HtmlToken::CData(mem::take(&mut self.data)))
                        }
                    }
                    _ => self.eof_tag(),
                };
                self.state = Eof;
                return Ok(token);
            };

            match self.state {
                Text(mode) => {
                    self.consume(c);
                    match (mode, c) {
                        (Mode::Data, '<') => self.state = DataLess,
                        (Mode::RcData | Mode::RawText | Mode::ScriptData, '<') => {
                            self.state = Less(mode);
                            if let Some(token) = self.text_token(mode) {
                                return Ok(Some(token));
                            }
                        }
                        (Mode::Data | Mode::RcData, '&')
                            if self.decode
                                || (mode == Mode::Data && self.tree.tracks_character_data()) =>
                        {
                            // Until the structural controller no longer needs character
                            // data, interpret references even when preserving raw output:
                            // decoded whitespace and visible text affect head/framesets.
                            if !self.decode {
                                self.data.push('&');
                            }
                            self.reference_start(if mode == Mode::Data {
                                ReferenceContext::Data
                            } else {
                                ReferenceContext::RcData
                            });
                        }
                        _ => {
                            // Data-state NUL is preserved; raw/RCDATA/script/plaintext replace it.
                            self.data.push(if c == '\0' && mode != Mode::Data {
                                '\u{fffd}'
                            } else {
                                c
                            });
                            if mode == Mode::Data && self.tree.tracks_character_data() {
                                self.tree.process_character_data(c.encode_utf8(&mut [0; 4]));
                            }
                        }
                    }
                }
                DataLess => {
                    // Avoid one data token per '<' in runs like "<<<<<<". The
                    // unconsumed next '<' can still begin real markup.
                    if c.is_ascii_alphabetic() || matches!(c, '!' | '/' | '?') {
                        self.state = TagOpen;
                        if let Some(token) = self.text_token(Mode::Data) {
                            return Ok(Some(token));
                        }
                    } else {
                        self.data.push('<');
                        self.tree.process_character_data("<");
                        self.state = Text(Mode::Data);
                    }
                }
                TagOpen => {
                    self.data.push('<');
                    match c {
                        '!' => {
                            self.consume(c);
                            self.data.push(c);
                            self.name.clear();
                            self.state = Declaration;
                        }
                        '?' => {
                            self.consume(c);
                            self.data.clear();
                            self.data.push('?');
                            self.state = BogusComment;
                        }
                        '/' => {
                            self.consume(c);
                            self.data.push(c);
                            self.state = EndTagOpen;
                        }
                        _ if c.is_ascii_alphabetic() => {
                            self.consume(c);
                            self.data.push(c);
                            self.name.push(c);
                            self.end_tag = false;
                            self.state = TagName;
                        }
                        _ => self.state = Text(Mode::Data),
                    }
                }
                EndTagOpen => {
                    self.consume(c);
                    self.data.push(c);
                    if c == '>' {
                        // Per HTML tokenization rules, "</>" emits nothing.
                        self.data.clear();
                        self.state = Text(Mode::Data);
                    } else if c.is_ascii_alphabetic() {
                        self.name.push(c);
                        self.end_tag = true;
                        self.state = TagName;
                    } else {
                        self.data.clear();
                        self.data.push(replace_null(c));
                        self.state = BogusComment;
                    }
                }
                TagName => {
                    self.consume(c);
                    self.data.push(c);
                    if space(c) || matches!(c, '/' | '>') {
                        mem::swap(&mut self.tag_name, &mut self.name);
                        if c == '>' {
                            return Ok(Some(self.tag_token(false)));
                        }
                        self.state = if c == '/' {
                            SelfClosing
                        } else {
                            BeforeAttribute
                        };
                    } else {
                        self.name.push(replace_null(c));
                    }
                }
                Less(mode) => {
                    self.data.push('<');
                    if c == '/' {
                        self.consume(c);
                        self.data.push('/');
                        self.name.clear();
                        self.state = EndOpen(match mode {
                            Mode::RcData => EndContext::RcData,
                            Mode::RawText => EndContext::RawText,
                            _ => EndContext::Script,
                        });
                    } else if mode == Mode::ScriptData && c == '!' {
                        self.consume(c);
                        self.data.push('!');
                        self.state = ScriptEscapeStart(false);
                    } else {
                        self.state = Text(mode);
                    }
                }
                EndOpen(context) => {
                    if c.is_ascii_alphabetic()
                        && (context != EndContext::Script || c.eq_ignore_ascii_case(&'s'))
                    {
                        self.consume(c);
                        self.data.push(c);
                        self.name.push(if context == EndContext::Script {
                            's'
                        } else {
                            c
                        });
                        self.state = EndName(context);
                    } else {
                        self.state = context.state();
                    }
                }
                EndName(context) => {
                    let active =
                        if matches!(context, EndContext::Script | EndContext::EscapedScript) {
                            "script"
                        } else {
                            &self.active_tag
                        };
                    if (space(c) || matches!(c, '/' | '>'))
                        && self.name.eq_ignore_ascii_case(active)
                    {
                        self.consume(c);
                        self.data.push(c);
                        self.end_tag = true;
                        mem::swap(&mut self.tag_name, &mut self.name);
                        if c == '>' {
                            return Ok(Some(self.tag_token(false)));
                        }
                        self.state = if c == '/' {
                            SelfClosing
                        } else {
                            BeforeAttribute
                        };
                    } else if c.is_ascii_alphabetic() {
                        self.consume(c);
                        self.data.push(c);
                        self.name.push(c);
                    } else {
                        // Reconsume the mismatch: the second '<' in "</styl</style>"
                        // must still be able to begin the appropriate end tag.
                        self.name.clear();
                        self.state = context.state();
                    }
                }
                Reference(context) => {
                    let allowed = if context == ReferenceContext::Attribute {
                        self.quote.unwrap_or('>')
                    } else {
                        '\0'
                    };
                    if (context == ReferenceContext::Attribute && c == allowed)
                        || !self
                            .entity
                            .push(c)
                            .expect("reference initialized with ampersand")
                    {
                        self.reference_end(context, Some(c));
                    } else {
                        self.consume(c);
                        // Preserve raw references for truncated tags and non-decoded data.
                        if context == ReferenceContext::Attribute || !self.decode {
                            self.data.push(c);
                        }
                        if c == ';' {
                            self.reference_end(context, Some(c));
                        }
                    }
                }
                ScriptEscapeStart(dash) => {
                    if c == '-' {
                        self.consume(c);
                        self.data.push(c);
                        self.state = if dash {
                            ScriptEscaped(2)
                        } else {
                            ScriptEscapeStart(true)
                        };
                    } else {
                        self.state = Text(Mode::ScriptData);
                    }
                }
                ScriptEscaped(dashes) | ScriptDoubleEscaped(dashes) => {
                    let double = matches!(self.state, ScriptDoubleEscaped(_));
                    self.consume(c);
                    if c == '<' {
                        self.state = if double {
                            ScriptDoubleLess
                        } else {
                            ScriptEscapedLess
                        };
                        let token = if double { None } else { self.script_token() };
                        self.data.push('<');
                        if token.is_some() {
                            return Ok(token);
                        }
                    } else {
                        self.data.push(replace_null(c));
                        self.state = if c == '>' && dashes == 2 {
                            Text(Mode::ScriptData)
                        } else if double {
                            ScriptDoubleEscaped(if c == '-' { (dashes + 1).min(2) } else { 0 })
                        } else {
                            ScriptEscaped(if c == '-' { (dashes + 1).min(2) } else { 0 })
                        };
                    }
                }
                ScriptEscapedLess => {
                    if c == '/' {
                        self.consume(c);
                        self.data.push(c);
                        self.name.clear();
                        self.state = EndOpen(EndContext::EscapedScript);
                    } else if c.is_ascii_alphabetic() {
                        self.consume(c);
                        self.data.push(c);
                        self.name.clear();
                        self.name.push(c);
                        self.state = ScriptDoubleWord(false);
                    } else {
                        self.state = ScriptEscaped(0);
                    }
                }
                ScriptDoubleLess => {
                    if c == '/' {
                        self.consume(c);
                        self.data.push(c);
                        self.name.clear();
                        self.state = ScriptDoubleWord(true);
                    } else {
                        self.state = ScriptDoubleEscaped(0);
                    }
                }
                ScriptDoubleWord(end) => {
                    if space(c) || matches!(c, '/' | '>') {
                        let script = self.name.eq_ignore_ascii_case("script");
                        self.consume(c);
                        self.data.push(c);
                        self.name.clear();
                        self.state = if script != end {
                            ScriptDoubleEscaped(0)
                        } else {
                            ScriptEscaped(0)
                        };
                    } else if c.is_ascii_alphabetic() {
                        self.consume(c);
                        self.data.push(c);
                        self.name.push(c);
                    } else {
                        self.name.clear();
                        self.state = if end {
                            ScriptDoubleEscaped(0)
                        } else {
                            ScriptEscaped(0)
                        };
                    }
                }
                BeforeAttribute | AfterAttribute => {
                    let after = self.state == AfterAttribute;
                    self.consume(c);
                    self.data.push(c);
                    match c {
                        _ if space(c) => {}
                        '/' => self.state = SelfClosing,
                        '>' => return Ok(Some(self.tag_token(false))),
                        '=' if after => self.state = BeforeValue,
                        _ => {
                            self.name.push(replace_null(c));
                            self.state = AttributeName;
                        }
                    }
                }
                AttributeName => {
                    self.consume(c);
                    self.data.push(c);
                    if space(c) || matches!(c, '/' | '=' | '>') {
                        self.attribute_name();
                        self.state = if c == '/' {
                            SelfClosing
                        } else if c == '=' {
                            BeforeValue
                        } else {
                            AfterAttribute
                        };
                        if c == '>' {
                            return Ok(Some(self.tag_token(false)));
                        }
                    } else {
                        self.name.push(replace_null(c));
                    }
                }
                BeforeValue => {
                    self.consume(c);
                    self.data.push(c);
                    match c {
                        _ if space(c) => {}
                        '"' | '\'' => {
                            self.quote = Some(c);
                            self.state = Value;
                        }
                        '&' => {
                            self.quote = None;
                            self.reference_start(ReferenceContext::Attribute);
                        }
                        '>' => return Ok(Some(self.tag_token(false))),
                        _ => {
                            self.quote = None;
                            self.name.push(replace_null(c));
                            self.state = Value;
                        }
                    }
                }
                Value => {
                    self.consume(c);
                    self.data.push(c);
                    if c == '&' {
                        self.reference_start(ReferenceContext::Attribute);
                    } else if self.quote == Some(c) {
                        self.attribute_value();
                        self.quote = None;
                        self.state = AfterValue;
                    } else if self.quote.is_none() && (space(c) || c == '>') {
                        self.attribute_value();
                        self.state = BeforeAttribute;
                        if c == '>' {
                            return Ok(Some(self.tag_token(false)));
                        }
                    } else {
                        self.name.push(replace_null(c));
                    }
                }
                AfterValue => match c {
                    _ if space(c) => {
                        self.consume(c);
                        self.data.push(c);
                        self.state = BeforeAttribute;
                    }
                    '/' => {
                        self.consume(c);
                        self.data.push(c);
                        self.state = SelfClosing;
                    }
                    '>' => {
                        self.consume(c);
                        return Ok(Some(self.tag_token(false)));
                    }
                    _ => self.state = BeforeAttribute,
                },
                SelfClosing => {
                    if c == '>' {
                        self.consume(c);
                        return Ok(Some(self.tag_token(true)));
                    }
                    // Reconsume rather than losing the 'o' of "<img/onerror=...>".
                    self.state = BeforeAttribute;
                }
                Declaration => {
                    let target = if self.name.starts_with('-') || (self.name.is_empty() && c == '-')
                    {
                        "--"
                    } else if self.name.starts_with('[') || (self.name.is_empty() && c == '[') {
                        "[CDATA["
                    } else {
                        "doctype"
                    };
                    let expected = target
                        .as_bytes()
                        .get(self.name.len())
                        .copied()
                        .map(char::from);
                    let matched = expected.is_some_and(|e| {
                        if target == "doctype" {
                            c.eq_ignore_ascii_case(&e)
                        } else {
                            c == e
                        }
                    });
                    if matched {
                        self.consume(c);
                        self.name.push(c);
                        if self.name.len() == target.len() {
                            if target == "--" {
                                self.name.clear();
                                self.data.clear();
                                self.state = CommentStart;
                            } else if target == "doctype" {
                                let mut doc = HtmlDocTypeToken::new();
                                doc.metadata.raw_tag_name = mem::take(&mut self.name);
                                self.doctype = Some(doc);
                                self.data.clear();
                                self.state = DocType;
                            } else if self.tree.is_in_foreign_content() {
                                self.data.clear();
                                self.name.clear();
                                self.state = CData;
                            } else {
                                self.data = mem::take(&mut self.name);
                                self.bang = true;
                                self.state = BogusComment;
                            }
                        }
                    } else {
                        // Only matching prefix characters are consumed; the mismatch
                        // may be the '>' terminating a bogus declaration such as "<!DOC>".
                        self.data = mem::take(&mut self.name);
                        self.bang = true;
                        self.state = BogusComment;
                    }
                }
                BogusComment => {
                    self.consume(c);
                    if c == '>' {
                        return Ok(Some(self.comment_token(true)));
                    }
                    self.data.push(replace_null(c));
                }
                CommentStart | CommentStartDash | Comment | CommentEndDash | CommentEnd
                | CommentEndBang => {
                    let state = self.state;
                    self.consume(c);
                    match state {
                        CommentStart => match c {
                            '-' => self.state = CommentStartDash,
                            '>' => return Ok(Some(self.comment_token(false))),
                            _ => {
                                self.name.push(replace_null(c));
                                self.state = Comment;
                            }
                        },
                        CommentStartDash | CommentEndDash => match c {
                            '-' => self.state = CommentEnd,
                            '>' if state == CommentStartDash => {
                                return Ok(Some(self.comment_token(false)));
                            }
                            _ => {
                                self.name.push('-');
                                self.name.push(replace_null(c));
                                self.state = Comment;
                            }
                        },
                        Comment => {
                            if c == '-' {
                                self.state = CommentEndDash;
                            } else {
                                self.name.push(replace_null(c));
                            }
                        }
                        CommentEnd => match c {
                            '>' => return Ok(Some(self.comment_token(false))),
                            '!' => self.state = CommentEndBang,
                            '-' => self.name.push('-'),
                            _ => {
                                self.name.push_str("--");
                                self.name.push(replace_null(c));
                                self.state = Comment;
                            }
                        },
                        _ => match c {
                            '>' => return Ok(Some(self.comment_token(false))),
                            '-' => {
                                self.name.push_str("--!");
                                self.state = CommentEndDash;
                            }
                            _ => {
                                self.name.push_str("--!");
                                self.name.push(replace_null(c));
                                self.state = Comment;
                            }
                        },
                    }
                }
                DocType => {
                    self.state = BeforeDocName;
                    if space(c) {
                        self.consume(c);
                    }
                }
                BeforeDocName => {
                    self.consume(c);
                    if c == '>' {
                        self.doctype.as_mut().unwrap().force_quirks_mode = true;
                        self.state = Text(Mode::Data);
                        return Ok(Some(self.doc_token()));
                    }
                    if !space(c) {
                        self.name.push(replace_null(c));
                        self.state = DocName;
                    }
                }
                DocName => {
                    self.consume(c);
                    if space(c) || c == '>' {
                        self.doc_name();
                        self.state = AfterDocName;
                        if c == '>' {
                            self.state = Text(Mode::Data);
                            return Ok(Some(self.doc_token()));
                        }
                    } else {
                        self.name.push(replace_null(c));
                    }
                }
                AfterDocName => {
                    self.consume(c);
                    if space(c) || c == '>' {
                        if !self.name.is_empty() {
                            self.doctype.as_mut().unwrap().force_quirks_mode = true;
                            self.name.clear();
                            self.state = BogusDocType;
                        }
                        if c == '>' {
                            self.state = Text(Mode::Data);
                            return Ok(Some(self.doc_token()));
                        }
                    } else {
                        self.name.push(c);
                        // PUBLIC and SYSTEM are six ASCII letters, not six UTF-8 bytes.
                        if self.name.chars().count() >= 6 {
                            let public = self.name.eq_ignore_ascii_case("public");
                            let system = self.name.eq_ignore_ascii_case("system");
                            if public || system {
                                let keyword = Some(mem::take(&mut self.name));
                                let doc = self.doctype.as_mut().unwrap();
                                if public {
                                    doc.metadata.public_keyword = keyword;
                                } else {
                                    doc.metadata.system_keyword = keyword;
                                }
                                self.state = BeforeDocIdentifier(public);
                            } else {
                                self.doctype.as_mut().unwrap().force_quirks_mode = true;
                                self.name.clear();
                                self.state = BogusDocType;
                            }
                        }
                    }
                }
                BeforeDocIdentifier(_) | AfterPublicIdentifier => {
                    let after_public = self.state == AfterPublicIdentifier;
                    let public = matches!(self.state, BeforeDocIdentifier(true));
                    self.consume(c);
                    if space(c) {
                        continue;
                    }
                    if matches!(c, '"' | '\'') {
                        self.quote = Some(c);
                        let quote = if c == '"' {
                            HtmlAttributeQuote::Double
                        } else {
                            HtmlAttributeQuote::Single
                        };
                        let doc = self.doctype.as_mut().unwrap();
                        if public {
                            doc.metadata.public_identifier_quote = quote;
                        } else {
                            doc.metadata.system_identifier_quote = quote;
                        }
                        self.doc_identifier(public);
                        self.state = DocIdentifier(public);
                    } else {
                        if !after_public || c != '>' {
                            self.doctype.as_mut().unwrap().force_quirks_mode = true;
                        }
                        if c == '>' {
                            self.state = Text(Mode::Data);
                            return Ok(Some(self.doc_token()));
                        }
                        self.state = BogusDocType;
                    }
                }
                DocIdentifier(public) => {
                    self.consume(c);
                    if Some(c) == self.quote || c == '>' {
                        self.doc_identifier(public);
                        self.quote = None;
                        if c == '>' {
                            self.doctype.as_mut().unwrap().force_quirks_mode = true;
                            self.state = Text(Mode::Data);
                            return Ok(Some(self.doc_token()));
                        }
                        self.state = if public {
                            AfterPublicIdentifier
                        } else {
                            AfterSystemIdentifier
                        };
                    } else {
                        self.name.push(replace_null(c));
                    }
                }
                AfterSystemIdentifier | BogusDocType => {
                    let bogus = self.state == BogusDocType;
                    self.consume(c);
                    if c == '>' {
                        self.state = Text(Mode::Data);
                        return Ok(Some(self.doc_token()));
                    }
                    if !bogus && !space(c) {
                        self.state = BogusDocType;
                    }
                }
                CData => {
                    self.consume(c);
                    if c == ']' {
                        if self.cdata_brackets == 2 {
                            self.data.push(']');
                        } else {
                            self.cdata_brackets += 1;
                        }
                    } else if c == '>' && self.cdata_brackets == 2 {
                        // Check the first terminator too: empty CDATA must terminate.
                        self.cdata_brackets = 0;
                        self.state = Text(Mode::Data);
                        self.tree.process_character_data(&self.data);
                        if !self.data.is_empty() {
                            return Ok(Some(HtmlToken::CData(mem::take(&mut self.data))));
                        }
                    } else {
                        for _ in 0..self.cdata_brackets {
                            self.data.push(']');
                        }
                        self.cdata_brackets = 0;
                        self.data.push(c);
                    }
                }
                Eof => unreachable!(),
            }
        }
    }
}

fn space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}
fn byte_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_input_preserves_a_split_utf8_scalar() {
        let input = "<p>abc\u{1f600}\r\nxyz</p>";
        let mut tokenizer = HtmlTokenizer::new();
        tokenizer.input.reserve_exact(64);
        let padding = tokenizer.input.capacity() - 7;
        tokenizer.input.extend(std::iter::repeat_n(b' ', padding));
        for _ in 0..padding {
            tokenizer.input.pop_front();
        }
        tokenizer.push(input.as_bytes()).unwrap();
        assert_eq!(tokenizer.input.as_slices().0.len(), 7);
        assert!(!tokenizer.input.as_slices().1.is_empty());
        tokenizer.finish();
        let mut reference = HtmlTokenizer::new();
        reference.push(input.as_bytes()).unwrap();
        reference.finish();
        loop {
            let actual = tokenizer.next_token().unwrap();
            assert_eq!(actual, reference.next_token().unwrap());
            assert_eq!(tokenizer.position(), reference.position());
            if actual == HtmlTokenizerResult::EndOfInput {
                break;
            }
        }
    }
}

fn update_position(position: &mut HtmlSourcePosition, text: &str) {
    // For short runs, count lines and UTF-8 leading bytes in a single pass
    // instead of paying for separate newline-search and scalar-count routines.
    if text.len() < 16 {
        for byte in text.bytes() {
            if byte == b'\n' {
                position.line += 1;
                position.column = 1;
            } else {
                position.column += u64::from(byte & 0xc0 != 0x80);
            }
        }
        position.offset += text.len() as u64;
        return;
    }
    if let Some(last) = memchr::memrchr(b'\n', text.as_bytes()) {
        position.line += memchr::memchr_iter(b'\n', &text.as_bytes()[..=last]).count() as u64;
        position.column = text[last + 1..].chars().count() as u64 + 1;
    } else {
        position.column += text.chars().count() as u64;
    }
    position.offset += text.len() as u64;
}

fn scan_tag_attributes(text: &str, attributes: &mut Vec<HtmlAttribute>) -> Option<(usize, bool)> {
    let bytes = text.as_bytes();
    let name_end = bytes
        .iter()
        .position(|&b| byte_space(b) || matches!(b, b'/' | b'>'))?;
    let mut index = name_end;
    loop {
        while byte_space(*bytes.get(index)?) {
            index += 1;
        }
        match bytes[index] {
            b'>' => return Some((name_end, false)),
            b'/' => return (bytes.get(index + 1) == Some(&b'>')).then_some((name_end, true)),
            b'=' => return None,
            _ => {}
        }
        let start = index;
        while !byte_space(*bytes.get(index)?) && !matches!(bytes[index], b'/' | b'=' | b'>') {
            index += 1;
        }
        let name = &text[start..index];
        while byte_space(*bytes.get(index)?) {
            index += 1;
        }
        let value = if bytes[index] == b'=' {
            index += 1;
            while byte_space(*bytes.get(index)?) {
                index += 1;
            }
            let quote = bytes[index];
            if matches!(quote, b'\'' | b'"') {
                index += 1;
                let end = index + memchr::memchr(quote, &bytes[index..])?;
                let value = &text[index..end];
                index = end + 1;
                Some(value)
            } else if quote == b'>' {
                None
            } else {
                let start = index;
                while !byte_space(*bytes.get(index)?) && bytes[index] != b'>' {
                    index += 1;
                }
                Some(&text[start..index])
            }
        } else {
            None
        };
        attributes.push(HtmlAttribute::from_tokenizer_str(name, value));
    }
}
fn replace_null(c: char) -> char {
    if c == '\0' { '\u{fffd}' } else { c }
}

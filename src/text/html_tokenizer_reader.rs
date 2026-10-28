// Adapted from MimeKit's Text/HtmlTokenizer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead};

use super::{
    HtmlToken, HtmlTokenizer, HtmlTokenizerOptions, HtmlTokenizerResult, InvalidHtmlElementDepth,
};

/// Synchronous `BufRead` adapter for the sans-I/O [`HtmlTokenizer`].
///
/// Copies bounded chunks into the engine only when it needs input. EOF is
/// signaled only by an empty successful source buffer; source errors do not
/// finish partial markup or UTF-8. Retry after recoverable I/O errors.
///
/// Input is strict UTF-8 without charset/BOM detection or newline normalization.
/// A future async reader can drive the same engine without changing tokenization.
///
/// ```
/// use mimecraft::text::{HtmlToken, HtmlTokenizerReader};
///
/// let mut reader = HtmlTokenizerReader::from_text("<p>Hello</p>");
/// while let Some(token) = reader.read_next_token()? {
///     if let HtmlToken::Data(data) = token {
///         assert_eq!(data.data(), "Hello");
///     }
/// }
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct HtmlTokenizerReader<R> {
    reader: R,
    tokenizer: HtmlTokenizer,
}

impl<'a> HtmlTokenizerReader<&'a [u8]> {
    /// Reads borrowed UTF-8 text through the synchronous adapter.
    pub fn from_text(text: &'a str) -> Self {
        Self::new(text.as_bytes())
    }
}

impl<R: BufRead> HtmlTokenizerReader<R> {
    /// Creates an adapter with default tokenizer options.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            tokenizer: HtmlTokenizer::new(),
        }
    }

    /// Creates an adapter with copied options.
    ///
    /// # Errors
    /// Returns [`InvalidHtmlElementDepth`] for a zero maximum depth.
    pub fn with_options(
        reader: R,
        options: HtmlTokenizerOptions,
    ) -> Result<Self, InvalidHtmlElementDepth> {
        Ok(Self {
            reader,
            tokenizer: HtmlTokenizer::with_options(options)?,
        })
    }

    /// Borrows the engine's options, position and structural state.
    pub fn tokenizer(&self) -> &HtmlTokenizer {
        &self.tokenizer
    }

    /// Borrows the engine for configuration. Do not supply or finish input here
    /// while the adapter is still reading the source.
    pub fn tokenizer_mut(&mut self) -> &mut HtmlTokenizer {
        &mut self.tokenizer
    }

    /// Borrows the source. Bytes already queued in the engine are not in it.
    pub fn get_ref(&self) -> &R {
        &self.reader
    }

    /// Returns source and engine, retaining queued bytes and partial-token state.
    pub fn into_parts(self) -> (R, HtmlTokenizer) {
        (self.reader, self.tokenizer)
    }

    /// Reads the next complete token, or `None` after source EOF.
    ///
    /// # Errors
    /// Propagates I/O errors; `Interrupted` is retried. Invalid UTF-8 is
    /// `InvalidData`, with the tokenizer error preserved as its source.
    pub fn read_next_token(&mut self) -> io::Result<Option<HtmlToken>> {
        loop {
            match self
                .tokenizer
                .next_token()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            {
                HtmlTokenizerResult::Token(token) => return Ok(Some(token)),
                HtmlTokenizerResult::EndOfInput => return Ok(None),
                HtmlTokenizerResult::NeedInput => {
                    let buffer = match self.reader.fill_buf() {
                        Ok(buffer) => buffer,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error),
                    };
                    if buffer.is_empty() {
                        self.tokenizer.finish();
                    } else {
                        let length = buffer.len().min(4096);
                        self.tokenizer
                            .push(&buffer[..length])
                            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                        self.reader.consume(length);
                    }
                }
            }
        }
    }
}

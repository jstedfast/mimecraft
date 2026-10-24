// Adapted from MimeKit's MimePart.WriteContent and MimeWriter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, Write};

use crate::{
    ContentEncoding, MimePart,
    encodings::{EncoderOptionsError, InvalidLineLength, NewLineFormat},
    filters::{Dos2UnixFilter, MimeFilter, Unix2DosFilter, encoder_filter_with_options},
    io::FilteredWriter,
};

/// Synchronous serialization settings shared by parts and MIME entity trees.
///
/// These do not edit headers, select a transfer encoding, or validate transport
/// constraints. Raw header values are not re-encoded or refolded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MimePartWriteOptions {
    /// Output newline format, defaulting to CRLF. `Mixed` is rejected.
    ///
    /// Applies to headers and nonbinary body output, not decoded bytes before
    /// Base64 encoding. Lone CR bytes follow the existing newline filters.
    pub new_line_format: NewLineFormat,
    /// Requested encoder line width, defaulting to 78; must be in 60..=998.
    ///
    /// Only newly encoded Base64/QP use this limit (capped at 76). UU has fixed
    /// width; existing encoded bodies and raw headers are not rewrapped.
    pub max_line_length: usize,
    /// Ensures a final root-body newline except for binary output. Defaults to false.
    ///
    /// Newly encoded nonempty bodies already end with a newline. Empty Base64
    /// and QP output receives one when requested; UU always has its terminator.
    /// For a multipart root this affects its epilogue, not child body contents.
    pub ensure_new_line: bool,
}

impl Default for MimePartWriteOptions {
    fn default() -> Self {
        Self {
            new_line_format: NewLineFormat::Dos,
            max_line_length: 78,
            ensure_new_line: false,
        }
    }
}

impl MimePartWriteOptions {
    pub(crate) fn validate(&self) -> io::Result<&'static [u8]> {
        let newline: &[u8] = match self.new_line_format {
            NewLineFormat::Dos => b"\r\n",
            NewLineFormat::Unix => b"\n",
            NewLineFormat::Mixed => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    EncoderOptionsError::InvalidNewLineFormat(NewLineFormat::Mixed),
                ));
            }
        };
        if !(60..=998).contains(&self.max_line_length) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                InvalidLineLength::new(self.max_line_length),
            ));
        }
        Ok(newline)
    }

    pub(crate) fn newline_filter(&self, ensure: bool) -> Box<dyn MimeFilter> {
        match self.new_line_format {
            NewLineFormat::Dos => Box::new(Unix2DosFilter::with_ensure_new_line(ensure)),
            NewLineFormat::Unix => Box::new(Dos2UnixFilter::with_ensure_new_line(ensure)),
            NewLineFormat::Mixed => {
                unreachable!("serialization validates options before filtering")
            }
        }
    }
}

impl<R: Read + Seek> MimePart<R> {
    /// Writes headers, a blank separator, and the body using default options.
    ///
    /// Uses the first Content-Transfer-Encoding header, or `Unspecified` if absent.
    /// Stored content with a different encoding is decoded and re-encoded
    /// incrementally; matching encodings avoid that conversion. Binary output
    /// preserves bytes, while other output uses the selected newline format.
    ///
    /// See [`Self::write_to_with_options`] for normalization and failure rules.
    ///
    /// # Errors
    ///
    /// Returns invalid metadata or I/O errors, potentially after partial output.
    pub fn write_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Writes a complete single part without changing its headers or stored bytes.
    ///
    /// Headers retain spelling, folding, duplicates, and malformed records using
    /// [`crate::HeaderList`]'s newline rules. Missing record terminators are added
    /// in output so records and the blank separator cannot run together.
    /// This is not header validation, transport preparation, or message writing.
    ///
    /// Newly UU-encoded bodies get `begin 0644 filename` and `end` wrappers.
    /// The filename comes from Content-Disposition, then Content-Type's `name`,
    /// then `unknown`; surrounding whitespace is trimmed. Control characters
    /// are rejected rather than allowing extra wrapper lines. Already stored UU
    /// is copied with newline conversion, without adding another wrapper.
    ///
    /// Source operations start at zero and rewind after successful copying,
    /// including before filter finalization. The destination is flushed, not
    /// closed; nested encoding writers still require their own `finish`.
    /// Neither source nor output is buffered in its entirety. Newline hints are
    /// ignored; signature-verification pass-through is not implemented.
    ///
    /// # Errors
    ///
    /// Invalid options return [`io::ErrorKind::InvalidInput`]; unknown transfer
    /// encoding, malformed required filename metadata, or unsafe UU filenames
    /// return [`io::ErrorKind::InvalidData`]. These checks precede any source I/O,
    /// output, or flush. Other content headers are not parsed unnecessarily.
    ///
    /// Seek/read/write/flush errors propagate and may leave partial or complete
    /// output. Source position is not restored after failed copying; failure
    /// during finalization may occur after it has been rewound. Retrying starts
    /// the whole part again, so clear or replace the destination first.
    ///
    /// ```
    /// use std::io::Cursor;
    /// use mimecraft::{ContentEncoding, MimeContent, MimePart};
    ///
    /// let mut part = MimePart::new(MimeContent::new(Cursor::new(b"hello")));
    /// part.set_content_transfer_encoding(ContentEncoding::Base64);
    /// let mut output = Vec::new();
    /// part.write_to(&mut output)?;
    /// assert!(output.ends_with(b"\r\n\r\naGVsbG8=\r\n"));
    /// assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn write_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        self.write_with_options(output, options, false)
    }

    /// Writes only the transfer-encoded content, using default options.
    ///
    /// Omits this part's headers and blank separator. This is serialization,
    /// not raw copying or decoding; see [`Self::write_content_to_with_options`].
    ///
    /// # Errors
    ///
    /// Returns metadata or I/O errors, potentially after partial output.
    pub fn write_content_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_content_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Writes the serialized body without headers or a header/body separator.
    ///
    /// Adapts MimeKit's content-only output. Uses the same transfer-encoding
    /// selection, conversion, newline rules, and UU wrappers as
    /// [`Self::write_to_with_options`]. Required metadata is still validated,
    /// including filenames when generating UU wrappers. Unrelated headers are
    /// neither parsed nor written.
    ///
    /// Unlike [`crate::MimeContent::write_to`], this honors the requested header
    /// encoding even when it differs from the stored content's encoding. Unlike
    /// [`crate::MimeContent::decode_to`], output is not necessarily decoded.
    /// Headers and stored bytes are unchanged. Successful copying rewinds the
    /// source; the destination is flushed, not closed. Nested encoding sinks
    /// still require their own finalization.
    ///
    /// # Errors
    ///
    /// Options and required metadata are checked before any source or
    /// destination I/O. Read, seek, write, and flush errors propagate, possibly
    /// after partial or complete output. No position restoration or output
    /// rollback occurs on failure. Retrying starts from the beginning.
    ///
    /// ```
    /// use std::io::Cursor;
    /// use mimecraft::{ContentEncoding, MimeContent, MimePart};
    ///
    /// let mut part = MimePart::new(MimeContent::new(Cursor::new(b"Cat")));
    /// part.set_content_transfer_encoding(ContentEncoding::Base64);
    /// let mut output = Vec::new();
    /// part.write_content_to(&mut output)?;
    /// assert_eq!(output, b"Q2F0\r\n");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn write_content_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        self.write_with_options(output, options, true)
    }

    fn write_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
        content_only: bool,
    ) -> io::Result<()> {
        let newline = options.validate()?;
        let encoding = self
            .content_transfer_encoding()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        // Stored encoding describes the actual source bytes, which may differ
        // from the requested header. Decode before applying the new encoding.
        let transcode = encoding != self.content().encoding();
        let encode = transcode
            && matches!(
                encoding,
                ContentEncoding::Base64
                    | ContentEncoding::QuotedPrintable
                    | ContentEncoding::UuEncode
            );
        let filter: Option<Box<dyn MimeFilter>> = if encode {
            Some(
                encoder_filter_with_options(
                    encoding,
                    options.max_line_length,
                    options.new_line_format,
                )
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
            )
        } else if encoding == ContentEncoding::Binary {
            None
        } else {
            Some(options.newline_filter(options.ensure_new_line))
        };
        let uu_name = if encode && encoding == ContentEncoding::UuEncode {
            Some(self.uu_file_name()?)
        } else {
            None
        };

        if !content_only {
            self.headers()
                .write_terminated(output, options.new_line_format)?;
            output.write_all(newline)?;
        }
        if let Some(name) = &uu_name {
            output.write_all(b"begin 0644 ")?;
            output.write_all(name.as_bytes())?;
            output.write_all(newline)?;
        }

        if let Some(filter) = filter {
            let mut filtered = FilteredWriter::new(&mut *output, filter);
            let count = if transcode {
                self.content_mut().decode_to(&mut filtered)?
            } else {
                self.content_mut().write_to(&mut filtered)?
            };
            filtered.finish()?;
            // Encoders terminate nonempty output; empty Base64/QP output
            // needs an explicit newline when requested. UU emits a terminator.
            if encode && count == 0 && options.ensure_new_line && uu_name.is_none() {
                output.write_all(newline)?;
            }
        } else if transcode {
            self.content_mut().decode_to(output)?;
        } else {
            // Do not alter binary content, even when ensuring a final newline.
            self.content_mut().write_to(output)?;
        }
        if uu_name.is_some() {
            output.write_all(b"end")?;
            output.write_all(newline)?;
        }
        output.flush()
    }

    fn uu_file_name(&self) -> io::Result<String> {
        let disposition = self
            .content_disposition()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let name = match disposition.as_ref().and_then(|value| value.file_name()) {
            Some(name) => name.to_owned(),
            None => self
                .content_type()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .and_then(|value| value.name().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned()),
        };
        if name.chars().any(char::is_control) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "UU filename must not contain control characters",
            ));
        }
        Ok(name.trim().to_owned())
    }
}

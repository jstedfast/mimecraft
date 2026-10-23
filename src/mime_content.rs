// Ported from MimeKit's MimeContent.cs and IMimeContent.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, Write};

use crate::{
    ContentEncoding,
    encodings::NewLineFormat,
    filters::{MimeFilter, decoder_filter},
    io::FilteredReader,
};

/// A seekable MIME body stream and the transfer encoding of its stored bytes.
///
/// Use [`new`](Self::new) for unencoded content, or
/// [`with_encoding`](Self::with_encoding) for already-encoded content.
/// [`write_to`](Self::write_to) copies those bytes unchanged;
/// [`open`](Self::open) and [`decode_to`](Self::decode_to) decode them.
/// No method encodes content, converts character sets, or normalizes newlines.
///
/// Operations start at absolute source position zero, not the position at
/// construction. Successful copies rewind the source to zero again; dropping
/// an opened reader does not. Decoding is incremental, without buffering the
/// entire body. The existing codecs' tolerant handling of malformed and
/// incomplete data is retained; this is not an encoding-validation API.
///
/// Owns the source, which may itself be a mutable reference. Rust ownership
/// replaces MimeKit's disposal checks: dropping this value drops an owned
/// source but does not close a borrowed one. An opened reader exclusively
/// borrows the source, preventing concurrent operations on the same content.
/// Async I/O and cancellation-token APIs are not implemented.
///
/// ```
/// use std::io::{Cursor, Read};
/// use mimecraft::{ContentEncoding, MimeContent};
///
/// let mut content = MimeContent::with_encoding(
///     Cursor::new(b"aGVsbG8=\r\n"),
///     ContentEncoding::Base64,
/// );
/// let mut decoded = Vec::new();
/// content.open()?.read_to_end(&mut decoded)?;
/// assert_eq!(decoded, b"hello");
///
/// let mut original = Vec::new();
/// assert_eq!(content.write_to(&mut original)?, 10);
/// assert_eq!(original, b"aGVsbG8=\r\n");
/// assert_eq!(content.get_ref().position(), 0);
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// Sources must support both reading and seeking at compile time:
///
/// ```compile_fail
/// use mimecraft::MimeContent;
///
/// // A byte-slice reader cannot seek; wrap it in std::io::Cursor instead.
/// let content = MimeContent::new(&b"body"[..]);
/// ```
#[derive(Debug)]
pub struct MimeContent<R> {
    inner: R,
    encoding: ContentEncoding,
    new_line_format: Option<NewLineFormat>,
}

impl<R: Read + Seek> MimeContent<R> {
    /// Wraps unencoded content with [`ContentEncoding::Unspecified`].
    ///
    /// Performs no I/O and does not change the source's current position.
    pub fn new(inner: R) -> Self {
        Self::with_encoding(inner, ContentEncoding::Unspecified)
    }

    /// Wraps bytes already stored in the specified transfer encoding.
    ///
    /// The encoding is a label, not a request to encode the source. Performs
    /// no I/O, validation, or automatic encoding/newline detection.
    pub fn with_encoding(inner: R, encoding: ContentEncoding) -> Self {
        Self {
            inner,
            encoding,
            new_line_format: None,
        }
    }

    /// Returns the transfer encoding of the stored source bytes.
    pub fn encoding(&self) -> ContentEncoding {
        self.encoding
    }

    /// Returns optional newline-format metadata, initially `None`.
    ///
    /// Like MimeKit's parser hint, this is not inferred or verified by content
    /// operations and never affects the bytes copied or decoded.
    pub fn new_line_format(&self) -> Option<NewLineFormat> {
        self.new_line_format
    }

    /// Sets or clears the newline hint without inspecting or changing the body.
    ///
    /// [`NewLineFormat::Mixed`] is valid metadata, unlike an encoder's requested
    /// output format. Setting this hint does not perform newline conversion.
    pub fn set_new_line_format(&mut self, new_line_format: Option<NewLineFormat>) {
        self.new_line_format = new_line_format;
    }

    /// Returns the underlying source.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Returns mutable access to the underlying source.
    ///
    /// Reading or seeking bypasses decoding; subsequent content operations
    /// still start at zero. If the bytes are modified, they must continue to
    /// match [`encoding`](Self::encoding). This does not update the newline hint.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Returns the source at its current position, without reading or rewinding.
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Rewinds the source and returns a fresh decoded [`Read`]/[`io::BufRead`] view.
    ///
    /// The reader borrows this content until dropped and does not support
    /// seeking. It reads lazily after the initial rewind. Dropping it discards
    /// unread decoded output and pending codec state without rewinding; source
    /// read-ahead may exceed the bytes delivered. Call `open` again to restart.
    ///
    /// Uses [`decoder_filter`]: Base64 and quoted-printable are tolerant body
    /// decoders, UU requires a `begin ` header, and other encodings pass through
    /// unchanged. Seven-bit/eight-bit labels do not validate byte ranges.
    ///
    /// # Errors
    ///
    /// Returns an initial seek error. Subsequent read errors are returned by
    /// the reader without clearing its buffered output or decoding state.
    pub fn open(&mut self) -> io::Result<FilteredReader<&mut R, Box<dyn MimeFilter>>> {
        self.inner.rewind()?;
        // Rewinding must also start fresh codec state: a previous reader may
        // have stopped in the middle of an encoded group or a UU begin line.
        Ok(FilteredReader::new(
            &mut self.inner,
            decoder_filter(self.encoding),
        ))
    }

    /// Copies stored bytes unchanged and returns the number of bytes copied.
    ///
    /// Rewinds before copying and again on success, matching MimeKit's
    /// `WriteTo`. Does not flush or close the destination. Uses [`io::copy`],
    /// including its handling of short I/O and interrupted reads/writes.
    ///
    /// # Errors
    ///
    /// Propagates seek, read, and write failures. A zero-byte write with output
    /// remaining returns [`io::ErrorKind::WriteZero`]. Read/write failures do not
    /// restore source position or roll back output. A final rewind can fail
    /// after all bytes have been written. Retrying starts over, not at the
    /// failure position, so the destination may need to be cleared or replaced.
    pub fn write_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<u64> {
        self.inner.rewind()?;
        let count = io::copy(&mut self.inner, output)?;
        // Restore the source for reuse after a successful copy, as MimeKit
        // does. Do not hide a failed rewind or replace an earlier I/O error.
        self.inner.rewind()?;
        Ok(count)
    }

    /// Copies decoded body bytes, flushes the destination, and returns their count.
    ///
    /// The count is decoded bytes, not encoded input bytes. This has the same
    /// decoder selection and tolerance as [`open`](Self::open). Rewinds before
    /// copying and after a successful copy, then flushes the destination.
    /// Neither endpoint is closed; existing destination bytes are not cleared.
    ///
    /// # Errors
    ///
    /// Propagates seek, read, write, and flush errors. As with
    /// [`write_to`](Self::write_to), failures may leave partial or complete
    /// output and retrying starts over. A failed final rewind prevents the
    /// flush; a flush failure leaves the source already rewound.
    pub fn decode_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<u64> {
        let count = io::copy(&mut self.open()?, output)?;
        self.inner.rewind()?;
        // MimeKit flushes its decoding output filter after WriteTo rewinds.
        // Our reader already finalized decoding at EOF; only sink flush remains.
        output.flush()?;
        Ok(count)
    }

    pub(crate) fn decode_to_vec_with_limit(&mut self, limit: usize) -> io::Result<Vec<u8>> {
        let mut output = LimitedBuffer {
            bytes: Vec::new(),
            limit,
        };
        self.decode_to(&mut output)?;
        Ok(output.bytes)
    }
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for LimitedBuffer {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.len() > self.limit - self.bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decoded content byte limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

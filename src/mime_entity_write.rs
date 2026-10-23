// Adapted from MimeKit's MimeWriter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, Read, Seek, Write},
    slice,
};

use crate::{
    MessagePart, MimeEntity, MimeMessage, MimePartWriteOptions, Multipart, MultipartError,
    io::FilteredWriter, message_part::check_message_part, multipart::checked_boundary,
};

impl<R: Read + Seek> MimeEntity<R> {
    /// Serializes this entity and its descendants using default options.
    ///
    /// Leaves use [`crate::MimePart::write_to`]; containers and attached messages
    /// share iterative traversal. No header or stored-body mutation occurs.
    ///
    /// # Errors
    ///
    /// Returns metadata or I/O errors, possibly after partial output.
    pub fn write_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Serializes this entity with explicit newline and encoding settings.
    ///
    /// See [`Multipart::write_to_with_options`] for tree and delimiter rules.
    /// A leaf root behaves exactly like [`crate::MimePart::write_to_with_options`].
    ///
    /// # Errors
    ///
    /// Returns invalid options, metadata, or I/O errors. Retry starts over.
    pub fn write_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        match self {
            Self::Part(part) => part.write_to_with_options(output, options),
            Self::Multipart(multipart) => multipart.write_to_with_options(output, options),
            Self::Message(part) => part.write_to_with_options(output, options),
        }
    }
}

impl<R: Read + Seek> Multipart<R> {
    /// Serializes a complete multipart tree using CRLF and default encoder widths.
    ///
    /// Uses an explicit traversal stack and existing single-part serializers.
    /// See [`Self::write_to_with_options`] for framing, validation, and errors.
    ///
    /// # Errors
    ///
    /// Returns metadata or I/O errors, possibly after partial output.
    pub fn write_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Writes headers, preamble, child entities, closing delimiter, and epilogue.
    ///
    /// Headers use the same normalization and missing-terminator repair as leaf
    /// serialization. Preamble/epilogue newlines are converted without charset
    /// decoding or folding. A nonempty preamble is newline-terminated before
    /// the first delimiter, including for an empty container. Closing delimiter
    /// lines always terminate; stored epilogue bytes exclude that line ending.
    /// Empty containers emit only a closing delimiter after any preamble.
    ///
    /// Every child is followed by a separate delimiter-owned newline. It is
    /// never borrowed from the child's body, even when that body ends in CRLF.
    /// This preserves binary suffixes and empty bodies. `ensure_new_line`
    /// affects the root epilogue only; children, including attached messages,
    /// are written without forcing extra body newlines. Encoder-generated
    /// endings are still retained.
    /// With LF output, a child ending in bare CR receives a CRLF delimiter
    /// prefix instead, avoiding an ambiguous CR+LF pair that would lose the
    /// body's CR when parsed. The marker line still uses the selected newline.
    ///
    /// Each container requires a valid multipart Content-Type and boundary.
    /// Equal or prefix-overlapping ancestor boundaries are rejected; siblings
    /// may reuse a boundary. Ancestors remain active across message wrappers.
    /// Multipart transfer encoding must be absent, 7bit,
    /// 8bit, or binary. No body or preamble/epilogue collision scanning, random
    /// boundary generation, transport preparation, or signature handling occurs.
    /// Callers must ensure delimiter values do not occur in content.
    /// Immediate message-part children of multipart/digest may omit Content-Type,
    /// relying on its message/rfc822 default. No header is inserted. This
    /// exception does not extend into attached bodies or other containers.
    ///
    /// Headers, bodies, and tree structure are unchanged. Traversal uses space
    /// proportional to depth, without recursion or whole-body buffering. Sources
    /// follow leaf serialization's rewind rules. The destination is flushed,
    /// never closed; nested encoding sinks still require their own finalization.
    ///
    /// # Errors
    ///
    /// Invalid options return [`io::ErrorKind::InvalidInput`] before any I/O.
    /// Invalid container metadata or ancestor boundary conflicts return
    /// [`io::ErrorKind::InvalidData`] before that container's headers are written.
    /// Validation proceeds in traversal order, not as an all-tree preflight:
    /// ancestor framing and earlier siblings may already have been written.
    /// Leaf metadata and I/O errors propagate unchanged. No output rollback or
    /// source-position restoration is attempted on failure; retry starts over.
    ///
    /// ```
    /// use mimecraft::{Multipart, TextPart};
    ///
    /// let mut mixed: Multipart = Multipart::new("mixed", "example-boundary")?;
    /// mixed.children_mut().push(TextPart::new("plain", "hello")?.into());
    /// let mut output = Vec::new();
    /// mixed.write_to(&mut output)?;
    /// assert!(output.ends_with(b"\r\n\r\nhello\r\n--example-boundary--\r\n"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn write_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        write_tree(
            Task::Multipart(self, options.ensure_new_line),
            output,
            options,
        )
    }
}

impl<R: Read + Seek> MessagePart<R> {
    /// Writes the wrapper and attached message with default CRLF formatting.
    ///
    /// See [`Self::write_to_with_options`] for validation and traversal rules.
    ///
    /// # Errors
    ///
    /// Returns metadata or I/O errors, possibly after partial output.
    pub fn write_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Writes wrapper headers, a separator, and the contained message, if any.
    ///
    /// An absent message emits no content; a present message with no body still
    /// emits its own headers and separator. The wrapper needs message/* or
    /// text/rfc822-headers Content-Type and an absent, 7bit, 8bit, or binary
    /// transfer encoding. Returned-header wrappers use the same traversal;
    /// an existing contained body is not discarded.
    /// Writing through a multipart/digest parent also accepts a missing
    /// Content-Type on its immediate message-part children. Standalone writing
    /// still requires an explicit type; add one before detaching such a child.
    /// Subtype-specific semantics and transport byte restrictions are not checked.
    /// Base64, quoted-printable, and UU wrappers are not implemented, including
    /// for message/global; use opaque [`crate::MimePart`] content for those.
    ///
    /// Contained messages use [`MimeMessage`] header ordering and normalization,
    /// with no automatic metadata or Bcc filtering. Options flow through to
    /// their body writers; binary leaves still preserve bytes. When this wrapper
    /// is the root, `ensure_new_line` reaches the innermost root body or epilogue.
    /// Multipart children never force an extra content newline.
    ///
    /// Message and multipart traversal share an explicit stack. Ancestor
    /// boundaries remain active inside attached messages, with the same overlap
    /// checks and independent delimiter newlines as [`Multipart`]. No whole
    /// message is buffered. Model drop and derived debugging still recurse.
    ///
    /// Headers and stored bytes are unchanged; successful leaf copying rewinds
    /// its source. Output is flushed, never closed.
    ///
    /// # Errors
    ///
    /// Invalid options fail with `InvalidInput` before I/O. Invalid wrapper
    /// metadata fails with `InvalidData` before its headers are written.
    /// Descendant errors propagate in traversal order, potentially after partial
    /// output. Nothing is rolled back; retrying writes the whole wrapper again.
    pub fn write_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        write_tree(
            Task::MessagePart(self, options.ensure_new_line, false),
            output,
            options,
        )
    }
}

pub(crate) fn write_message<R: Read + Seek, W: Write + ?Sized>(
    message: &mut MimeMessage<R>,
    output: &mut W,
    options: &MimePartWriteOptions,
) -> io::Result<()> {
    write_tree(
        Task::Message(message, options.ensure_new_line),
        output,
        options,
    )
}

fn write_tree<R: Read + Seek, W: Write + ?Sized>(
    root: Task<'_, R>,
    output: &mut W,
    options: &MimePartWriteOptions,
) -> io::Result<()> {
    let newline = options.validate()?;
    let mut tracked = EndingWriter {
        inner: output,
        last: None,
    };
    let output = &mut tracked;
    let mut stack = vec![root];

    while let Some(task) = stack.pop() {
        match task {
            Task::Entity(entity, ensure_new_line, digest_child) => match entity {
                MimeEntity::Part(part) => part.write_to_with_options(
                    output,
                    &MimePartWriteOptions {
                        ensure_new_line,
                        ..*options
                    },
                )?,
                MimeEntity::Multipart(multipart) => {
                    stack.push(Task::Multipart(multipart, ensure_new_line));
                }
                MimeEntity::Message(part) => {
                    stack.push(Task::MessagePart(part, ensure_new_line, digest_child));
                }
            },
            Task::MessagePart(part, ensure_new_line, digest_child) => {
                check_message_part(part, digest_child)?;
                part.headers()
                    .write_terminated(output, options.new_line_format)?;
                output.write_all(newline)?;
                if let Some(message) = part.message_mut() {
                    // The embedded message inherits its wrapper's final-newline
                    // policy, not the top-level formatting option.
                    stack.push(Task::Message(message, ensure_new_line));
                }
            }
            Task::Message(message, ensure_new_line) => {
                message
                    .headers()
                    .write_terminated(output, options.new_line_format)?;
                if let Some(body) = message.body_mut() {
                    stack.push(Task::Entity(body, ensure_new_line, false));
                } else {
                    output.write_all(newline)?;
                }
            }
            Task::Multipart(multipart, ensure_new_line) => {
                let boundary = checked_boundary(multipart)?;
                let digest = multipart
                    .content_type()
                    .map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            MultipartError::ContentType(error),
                        )
                    })?
                    .is_some_and(|value| value.is_mime_type("multipart", "digest"));
                for ancestor in &stack {
                    if let Task::Children(frame) = ancestor
                        && (boundary.starts_with(&frame.boundary)
                            || frame.boundary.starts_with(&boundary))
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "multipart boundary overlaps an ancestor boundary",
                        ));
                    }
                }
                multipart
                    .headers()
                    .write_terminated(output, options.new_line_format)?;
                output.write_all(newline)?;
                write_text(output, multipart.preamble(), options, true)?;
                let (children, epilogue) = multipart.children_and_epilogue();
                stack.push(Task::Children(Frame {
                    children: children.iter_mut(),
                    boundary,
                    epilogue,
                    after_child: false,
                    ensure_new_line,
                    digest,
                }));
            }
            Task::Children(mut frame) => {
                // The newline preceding a boundary belongs to that
                // delimiter, not to the previous child's content. Never
                // consume an existing body newline, especially for binary.
                if frame.after_child {
                    let delimiter_newline = if newline == b"\n" && output.last == Some(b'\r') {
                        b"\r\n".as_slice()
                    } else {
                        newline
                    };
                    output.write_all(delimiter_newline)?;
                }
                output.write_all(b"--")?;
                output.write_all(frame.boundary.as_bytes())?;
                if let Some(child) = frame.children.next() {
                    output.write_all(newline)?;
                    frame.after_child = true;
                    let digest_child = frame.digest;
                    stack.push(Task::Children(frame));
                    stack.push(Task::Entity(child, false, digest_child));
                } else {
                    output.write_all(b"--")?;
                    output.write_all(newline)?;
                    write_text(output, frame.epilogue, options, frame.ensure_new_line)?;
                }
            }
        }
    }
    output.flush()
}

struct Frame<'a, R> {
    children: slice::IterMut<'a, MimeEntity<R>>,
    boundary: String,
    epilogue: &'a [u8],
    after_child: bool,
    ensure_new_line: bool,
    digest: bool,
}

struct EndingWriter<'a, W: ?Sized> {
    inner: &'a mut W,
    last: Option<u8>,
}

impl<W: Write + ?Sized> Write for EndingWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(bytes)?;
        if count > 0 {
            self.last = Some(bytes[count - 1]);
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

enum Task<'a, R> {
    Entity(&'a mut MimeEntity<R>, bool, bool),
    MessagePart(&'a mut MessagePart<R>, bool, bool),
    Message(&'a mut MimeMessage<R>, bool),
    Multipart(&'a mut Multipart<R>, bool),
    Children(Frame<'a, R>),
}

fn write_text<W: Write + ?Sized>(
    output: &mut W,
    bytes: &[u8],
    options: &MimePartWriteOptions,
    ensure_new_line: bool,
) -> io::Result<()> {
    if !bytes.is_empty() {
        let mut filtered = FilteredWriter::new(output, options.newline_filter(ensure_new_line));
        filtered.write_all(bytes)?;
        filtered.finish()?;
    }
    Ok(())
}

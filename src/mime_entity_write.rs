// Adapted from MimeKit's MimeWriter.cs and MimeAnonymizer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, Read, Seek, Write},
    slice,
};

use crate::{
    MessagePart, MimeEntity, MimeMessage, MimePartWriteOptions, Multipart, MultipartError,
    anonymize, io::FilteredWriter, message_part::check_message_part, multipart::checked_boundary,
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

    /// Serializes only this entity's content using default options.
    ///
    /// Omits root headers and their separator, not descendant headers.
    /// See [`Self::write_content_to_with_options`] for variant-specific behavior.
    ///
    /// # Errors
    ///
    /// Returns metadata or I/O errors, possibly after partial output.
    pub fn write_content_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_content_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Serializes content without this entity's headers or blank separator.
    ///
    /// Leaves use [`crate::MimePart::write_content_to_with_options`], retaining
    /// transfer encoding rather than simply decoding or copying stored bytes.
    /// Multiparts retain preambles, complete children, delimiters, and epilogues.
    /// Message wrappers retain the complete enclosed message. Only the root's
    /// headers are omitted; required metadata validation and all other writer
    /// behavior remain unchanged.
    ///
    /// Output is streamed and flushed, not closed. Successful body copying
    /// rewinds sources, without changing headers, stored bytes, or tree shape.
    ///
    /// # Errors
    ///
    /// Options fail before I/O. Required metadata and source/destination errors
    /// propagate using the full writer's traversal-order validation; partial
    /// output is possible. Retrying starts over.
    pub fn write_content_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        match self {
            Self::Part(part) => part.write_content_to_with_options(output, options),
            Self::Multipart(multipart) => multipart.write_content_to_with_options(output, options),
            Self::Message(part) => part.write_content_to_with_options(output, options),
        }
    }
}

impl<R: Read + Seek> Multipart<R> {
    /// Serializes a multipart tree using CRLF and default encoder widths.
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
    /// Headers use the leaf writer's normalization and parsed-separator rules.
    /// Preamble/epilogue newlines are converted without charset decoding or
    /// folding. Nonempty preambles are terminated before emitted delimiters.
    /// Authored containers emit a terminated closing delimiter even when empty.
    /// Unchanged parsed containers retain raw marker spelling, padding, original
    /// marker line endings, and missing closing markers or final terminators.
    ///
    /// A separate delimiter-owned newline follows each child unless its parsed
    /// empty-body or truncated framing already owns that newline. Binary body
    /// suffixes are never consumed. `ensure_new_line` applies to the epilogue,
    /// or the final child/trailing marker of a truncated container. Binary tails
    /// remain unchanged. Encoder-generated endings are retained.
    /// With LF output, a child ending in bare CR receives a CRLF delimiter
    /// prefix instead, avoiding an ambiguous CR+LF pair that would lose the
    /// body's CR when parsed. Generated marker lines use the selected newline.
    ///
    /// Authored or edited containers require a valid multipart type and boundary.
    /// Unchanged parsed metadata retains the reader's recovery interpretation,
    /// including missing boundaries whose content remains entirely preamble.
    /// Equal or prefix-overlapping ancestor boundaries are preserved only when
    /// this container and every active multipart ancestor retain original parsed
    /// framing. Otherwise they are rejected, including after taking mutable
    /// children or moving a parsed subtree under an authored container. Siblings
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
    /// [`io::ErrorKind::InvalidData`] before that container's headers are written,
    /// unless already emitted as part of a message's interleaved header section.
    /// Validation proceeds in traversal order, not as an all-tree preflight:
    /// ancestor framing and earlier siblings may already have been written.
    /// Leaf metadata and I/O errors propagate unchanged. No output rollback or
    /// source-position restoration is attempted on failure; retry starts over.
    ///
    /// ```
    /// use mimecraft::{Multipart, TextPart};
    ///
    /// let mut mixed: Multipart = Multipart::with_boundary("mixed", "example-boundary")?;
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
            Task::Multipart(self, options.ensure_new_line, false),
            output,
            options,
            None,
        )
    }

    /// Writes multipart content without root headers, using default options.
    ///
    /// See [`Self::write_content_to_with_options`] for framing and validation.
    ///
    /// # Errors
    ///
    /// Returns metadata or I/O errors, possibly after partial output.
    pub fn write_content_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_content_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Writes the preamble, complete children, closing delimiter, and epilogue.
    ///
    /// Omits only this multipart's headers and blank separator. Child headers
    /// and independent delimiter-owned newlines are retained. All options,
    /// boundary/transfer-encoding checks, iterative traversal, source rewind,
    /// and destination flush rules of [`Self::write_to_with_options`] apply.
    /// Ancestor boundary checks include this root even though its headers
    /// are omitted. No content is buffered in its entirety.
    ///
    /// # Errors
    ///
    /// Invalid options or root metadata fail before I/O. Descendant metadata
    /// and I/O errors may leave partial output. Nothing is rolled back; retrying
    /// starts at the beginning.
    pub fn write_content_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        write_tree(
            Task::Multipart(self, options.ensure_new_line, true),
            output,
            options,
            None,
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
    /// Unchanged parsed empty/unknown transfer-encoding headers retain the
    /// reader's unspecified interpretation; authored or edited invalid values
    /// remain errors.
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
    /// metadata fails with `InvalidData` before its headers are written, unless
    /// already emitted as part of a message's interleaved header section.
    /// Descendant errors propagate in traversal order, potentially after partial
    /// output. Nothing is rolled back; retrying writes the whole wrapper again.
    pub fn write_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        write_tree(
            Task::MessagePart(self, options.ensure_new_line, false, false),
            output,
            options,
            None,
        )
    }

    /// Writes the enclosed message without wrapper headers, using default options.
    ///
    /// See [`Self::write_content_to_with_options`] for validation and framing.
    ///
    /// # Errors
    ///
    /// Returns metadata or I/O errors, possibly after partial output.
    pub fn write_content_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_content_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Writes the complete enclosed message without this wrapper's header section.
    ///
    /// The message retains its headers, root-body headers, separator, and body.
    /// An absent message writes nothing but still flushes; a present empty
    /// message writes its own separator. Descendant wrapper and multipart
    /// headers are retained. Bcc is not filtered and no metadata is generated.
    ///
    /// All wrapper validation and iterative traversal rules of
    /// [`Self::write_to_with_options`] apply, including the explicit standalone
    /// Content-Type requirement and rejection of encoded wrappers. Omitting
    /// headers does not enable previously unsupported transfer encodings.
    /// `ensure_new_line` still reaches the enclosed message's root body.
    ///
    /// # Errors
    ///
    /// Options and root wrapper metadata fail before I/O; descendant metadata
    /// and I/O errors may leave partial output. Retrying starts over, without
    /// restoring source positions on failure.
    pub fn write_content_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        write_tree(
            Task::MessagePart(self, options.ensure_new_line, false, true),
            output,
            options,
            None,
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
        None,
    )
}

pub(crate) fn write_masked_entity<R: Read + Seek, W: Write + ?Sized>(
    entity: &mut MimeEntity<R>,
    output: &mut W,
    options: &anonymize::WriteOptions,
    content_only: bool,
) -> io::Result<()> {
    options.validate()?;
    write_tree(
        Task::Entity(entity, options.part.ensure_new_line, false, content_only),
        output,
        &options.part,
        Some(options),
    )
}

pub(crate) fn write_masked_message<R: Read + Seek, W: Write + ?Sized>(
    message: &mut MimeMessage<R>,
    output: &mut W,
    options: &anonymize::WriteOptions,
) -> io::Result<()> {
    options.validate()?;
    write_tree(
        Task::Message(message, options.part.ensure_new_line),
        output,
        &options.part,
        Some(options),
    )
}

fn write_tree<R: Read + Seek, W: Write + ?Sized>(
    root: Task<'_, R>,
    output: &mut W,
    options: &MimePartWriteOptions,
    masking: Option<&anonymize::WriteOptions>,
) -> io::Result<()> {
    let newline = options.validate()?;
    let mut tracked = EndingWriter {
        inner: output,
        last: None,
    };
    let output = &mut tracked;
    // Both MimeKit writers use an explicit stack so nested multiparts and
    // messages cannot exhaust the call stack. Only multiparts retain a frame:
    // a wrapper's embedded message and a message's body are their final tasks.
    let mut stack = vec![root];

    while let Some(task) = stack.pop() {
        match task {
            Task::Entity(entity, ensure_new_line, digest_child, content_only) => {
                if let Some(masking) = masking
                    && !content_only
                {
                    // The anonymizer writes headers before validating content,
                    // unlike standalone ordinary entity serialization.
                    anonymize::write_headers_with_options(
                        entity.headers(),
                        output,
                        &masking.headers,
                        options.new_line_format,
                    )?;
                    output.write_all(newline)?;
                }
                let content_only = content_only || masking.is_some();
                match entity {
                    MimeEntity::Part(part) => {
                        let options = MimePartWriteOptions {
                            ensure_new_line,
                            ..*options
                        };
                        if let Some(masking) = masking {
                            anonymize::write_leaf_content(part, output, masking, &options)?;
                        } else if content_only {
                            part.write_content_to_with_options(output, &options)?;
                        } else {
                            part.write_to_with_options(output, &options)?;
                        }
                    }
                    MimeEntity::Multipart(multipart) => {
                        stack.push(Task::Multipart(multipart, ensure_new_line, content_only));
                    }
                    MimeEntity::Message(part) => {
                        stack.push(Task::MessagePart(
                            part,
                            ensure_new_line,
                            digest_child,
                            content_only,
                        ));
                    }
                }
            }
            Task::MessagePart(part, ensure_new_line, digest_child, content_only) => {
                check_message_part(part, digest_child)?;
                // Content-only omits the wrapper, not the embedded message's
                // header section. The message is still written in full.
                if !content_only {
                    crate::mime_entity::write_entity_headers(
                        part.headers(),
                        part.parsed,
                        output,
                        options,
                    )?;
                }
                if let Some(message) = part.message.as_deref_mut() {
                    // The embedded message inherits its wrapper's final-newline
                    // policy, not the top-level formatting option.
                    stack.push(Task::Message(message, ensure_new_line));
                }
            }
            Task::Message(message, ensure_new_line) => {
                if let Some(masking) = masking {
                    anonymize::write_message_headers_with_options(
                        message,
                        output,
                        &masking.headers,
                        options.new_line_format,
                    )?;
                } else {
                    message.write_headers_to_with_options(output, options)?;
                }
                if let Some(body) = message.body_mut() {
                    // The root body's headers were merged with the message's.
                    // Omit only that section; descendants still write in full.
                    stack.push(Task::Entity(body, ensure_new_line, false, true));
                }
            }
            Task::Multipart(multipart, ensure_new_line, content_only) => {
                let original_framing = multipart.original_framing();
                if original_framing.is_some_and(|framing| framing.boundary.is_none()) {
                    // Without a boundary parameter, the reader retains all
                    // bytes as preamble. Do not invent a marker on output.
                    if !content_only {
                        crate::mime_entity::write_entity_headers(
                            multipart.headers(),
                            multipart.parsed,
                            output,
                            options,
                        )?;
                    }
                    write_text(
                        output,
                        multipart.preamble(),
                        options,
                        ensure_new_line,
                        masking,
                    )?;
                    continue;
                }
                let boundary = if let Some(framing) = original_framing {
                    framing.boundary.clone().expect("handled missing boundary")
                } else {
                    checked_boundary(multipart)?
                };
                let digest = if let Some(framing) = original_framing {
                    crate::ContentType::parse_for_reader(&framing.content_type)
                        .0
                        .is_mime_type("multipart", "digest")
                } else {
                    multipart
                        .content_type()
                        .map_err(|error| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                MultipartError::ContentType(error),
                            )
                        })?
                        .is_some_and(|value| value.is_mime_type("multipart", "digest"))
                };
                // MimeKit writes raw boundary markers byte-for-byte as parsed.
                // Preserve recovered overlaps only while the entire multipart
                // ancestry retains its original framing. Editing an intervening
                // container can move a parsed subtree into a conflicting context.
                let mut parsed_ancestry = original_framing.is_some();
                let mut overlaps = false;
                for ancestor in &stack {
                    if let Task::Children(frame) = ancestor {
                        parsed_ancestry &= frame.framing.is_some();
                        overlaps |= boundary.starts_with(&frame.boundary)
                            || frame.boundary.starts_with(&boundary);
                    }
                }
                if overlaps && !parsed_ancestry {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "multipart boundary overlaps an ancestor boundary",
                    ));
                }
                if !content_only {
                    crate::mime_entity::write_entity_headers(
                        multipart.headers(),
                        original_framing.and(multipart.parsed),
                        output,
                        options,
                    )?;
                }
                let framing = original_framing;
                let preserve_framing = framing.is_some();
                let truncated = framing.is_some_and(|framing| framing.end.is_none());
                write_text(
                    output,
                    multipart.preamble(),
                    options,
                    !truncated || !multipart.children().is_empty() || ensure_new_line,
                    masking,
                )?;
                let (children, epilogue, framing) = multipart.children_and_epilogue();
                let framing = framing.filter(|_| preserve_framing);
                stack.push(Task::Children(Frame {
                    children: children.iter_mut(),
                    boundary,
                    epilogue,
                    after_child: false,
                    ensure_new_line,
                    digest,
                    framing,
                    index: 0,
                }));
            }
            Task::Children(mut frame) => {
                // The newline preceding a boundary belongs to that
                // delimiter, not to the previous child's content. Never
                // consume an existing body newline, especially for binary.
                let raw_boundary = frame
                    .framing
                    .and_then(|framing| framing.boundaries.get(frame.index));
                let truncated = frame.framing.is_some_and(|framing| framing.end.is_none());
                let has_marker =
                    !frame.children.as_slice().is_empty() || raw_boundary.is_some() || !truncated;
                if frame.after_child && has_marker {
                    let delimiter_newline = if newline == b"\n" && output.last == Some(b'\r') {
                        b"\r\n".as_slice()
                    } else {
                        newline
                    };
                    output.write_all(delimiter_newline)?;
                }
                if let Some(child) = frame.children.next() {
                    if let Some(raw) = raw_boundary {
                        write_raw_boundary(output, raw, options, masking.is_some())?;
                    } else {
                        output.write_all(b"--")?;
                        output.write_all(frame.boundary.as_bytes())?;
                        output.write_all(newline)?;
                    }
                    frame.after_child = !child.ends_with_delimiter_newline();
                    frame.index += 1;
                    let ensure_child = frame.ensure_new_line
                        && truncated
                        && frame.children.as_slice().is_empty()
                        && frame
                            .framing
                            .is_some_and(|framing| framing.boundaries.len() == frame.index);
                    let digest_child = frame.digest;
                    stack.push(Task::Children(frame));
                    // As in MimeKit, every multipart child starts a complete
                    // entity, even when its parent's headers were omitted.
                    stack.push(Task::Entity(child, ensure_child, digest_child, false));
                } else if truncated {
                    // No closing boundary or epilogue was present. Preserve
                    // a trailing opening marker even when it has no MIME part.
                    if let Some(raw) = raw_boundary {
                        write_raw_boundary(output, raw, options, masking.is_some())?;
                    }
                    if frame.ensure_new_line
                        && (raw_boundary.is_some() || !frame.after_child)
                        && output.last.is_some_and(|last| last != b'\n')
                    {
                        output.write_all(newline)?;
                    }
                } else {
                    if let Some(raw) = frame.framing.and_then(|framing| framing.end.as_ref()) {
                        write_raw_boundary(output, raw, options, masking.is_some())?;
                    } else {
                        output.write_all(b"--")?;
                        output.write_all(frame.boundary.as_bytes())?;
                        output.write_all(b"--")?;
                        output.write_all(newline)?;
                    }
                    write_text(
                        output,
                        frame.epilogue,
                        options,
                        frame.ensure_new_line,
                        masking,
                    )?;
                    if frame.ensure_new_line && output.last.is_some_and(|last| last != b'\n') {
                        output.write_all(newline)?;
                    }
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
    framing: Option<&'a crate::multipart::ParsedMultipart>,
    index: usize,
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
    Entity(&'a mut MimeEntity<R>, bool, bool, bool),
    MessagePart(&'a mut MessagePart<R>, bool, bool, bool),
    Message(&'a mut MimeMessage<R>, bool),
    Multipart(&'a mut Multipart<R>, bool, bool),
    Children(Frame<'a, R>),
}

fn write_raw_boundary<W: Write + ?Sized>(
    output: &mut W,
    bytes: &[u8],
    options: &MimePartWriteOptions,
    normalize: bool,
) -> io::Result<()> {
    // MimeKit retains parsed marker bytes, including padding and line endings.
    // The anonymizer instead retains its existing output-newline policy.
    if normalize {
        write_text(output, bytes, options, false, None)
    } else {
        output.write_all(bytes)
    }
}

fn write_text<W: Write + ?Sized>(
    output: &mut W,
    bytes: &[u8],
    options: &MimePartWriteOptions,
    ensure_new_line: bool,
    masking: Option<&anonymize::WriteOptions>,
) -> io::Result<()> {
    if masking.is_some() {
        return anonymize::write_bytes_with_options(
            bytes,
            output,
            options.new_line_format,
            ensure_new_line,
        );
    }
    if !bytes.is_empty() {
        let mut filtered = FilteredWriter::new(output, options.newline_filter(ensure_new_line));
        filtered.write_all(bytes)?;
        filtered.finish()?;
    }
    Ok(())
}

// Adapted from MimeKit's MimeEntity.cs, Multipart.cs, MessagePart.cs, and MimeMessage.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Read, Seek},
    slice,
};

use crate::{
    EncodingConstraint, MessagePart, MimeEntity, MimeMessage, MimePartWriteOptions, MimeVersion,
    MimeVersionParseError, Multipart, PartPrepareError,
};

/// Preparation stopped at invalid options, a message version, or a leaf failure.
///
/// Tree preparation is not transactional. Earlier leaf edits and message
/// MIME-Version insertions remain when a later operation fails.
#[derive(Debug)]
pub enum MimePrepareError {
    /// Write options were rejected before traversal or any mutation.
    InvalidOptions(io::Error),
    /// A message with a body has an invalid first MIME-Version.
    MimeVersion(MimeVersionParseError),
    /// A leaf could not be prepared; its headers remain unchanged.
    Part(PartPrepareError),
}

impl fmt::Display for MimePrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptions(error) => error.fmt(f),
            Self::MimeVersion(error) => error.fmt(f),
            Self::Part(error) => error.fmt(f),
        }
    }
}

impl Error for MimePrepareError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidOptions(error) => Some(error),
            Self::MimeVersion(error) => Some(error),
            Self::Part(error) => Some(error),
        }
    }
}

impl<R: Read + Seek> MimeEntity<R> {
    /// Prepares this entity and its descendants using default write options.
    ///
    /// See [`Self::prepare_with_options`] for traversal and partial-failure rules.
    ///
    /// # Errors
    ///
    /// Returns [`MimePrepareError`], possibly after earlier nodes were changed.
    pub fn prepare(&mut self, constraint: EncodingConstraint) -> Result<(), MimePrepareError> {
        self.prepare_with_options(constraint, &MimePartWriteOptions::default())
    }

    /// Prepares leaf encodings throughout this tree, including attached messages.
    ///
    /// Traverses depth-first in child order with an explicit stack proportional
    /// to nesting depth. Leaves use [`crate::MimePart::prepare_with_options`].
    /// Multiparts only delegate to children; message wrappers only delegate to
    /// their enclosed message, if any. Container headers, preambles, epilogues,
    /// and tree structure are neither validated nor changed. Dispatch follows
    /// enum variants rather than guessing structure from raw Content-Type.
    ///
    /// Enclosed messages use [`MimeMessage::prepare_with_options`]'s version
    /// rules. This does not add a message-level version to a bare entity root.
    /// Pass the same options when writing: no encoder-width state is retained.
    /// Stored bytes and newline hints are unchanged, but content analysis can
    /// read and rewind sources. Already-safe encodings are trusted, not verified.
    /// Raw headers are not refolded and container encodings are not repaired;
    /// successful preparation does not guarantee transport-safe serialization.
    ///
    /// # Errors
    ///
    /// Options are validated before traversal, even for empty trees. The first
    /// version, leaf metadata, or I/O error stops traversal. Earlier leaf edits
    /// and message-version insertions remain; later siblings are untouched.
    /// Source positioning follows [`crate::MimePart::best_encoding`].
    /// Retrying traverses again with no rollback or cached recommendations.
    pub fn prepare_with_options(
        &mut self,
        constraint: EncodingConstraint,
        options: &MimePartWriteOptions,
    ) -> Result<(), MimePrepareError> {
        prepare_tree(Task::Entity(self), constraint, options)
    }
}

impl<R: Read + Seek> Multipart<R> {
    /// Prepares all children, including attached messages, using default options.
    ///
    /// # Errors
    ///
    /// Returns [`MimePrepareError`], retaining changes to earlier nodes.
    pub fn prepare(&mut self, constraint: EncodingConstraint) -> Result<(), MimePrepareError> {
        self.prepare_with_options(constraint, &MimePartWriteOptions::default())
    }

    /// Prepares descendants without inspecting or changing this container.
    ///
    /// See [`MimeEntity::prepare_with_options`] for iterative traversal, explicit
    /// writer settings, and failure semantics. This also visits children of
    /// malformed containers; writing still validates their metadata separately.
    ///
    /// # Errors
    ///
    /// Invalid options precede traversal. A descendant failure retains edits
    /// already made and leaves later siblings untouched.
    pub fn prepare_with_options(
        &mut self,
        constraint: EncodingConstraint,
        options: &MimePartWriteOptions,
    ) -> Result<(), MimePrepareError> {
        prepare_tree(Task::Multipart(self), constraint, options)
    }
}

impl<R: Read + Seek> MessagePart<R> {
    /// Prepares the enclosed message, if present, using default write options.
    ///
    /// # Errors
    ///
    /// Returns [`MimePrepareError`], possibly after earlier edits.
    pub fn prepare(&mut self, constraint: EncodingConstraint) -> Result<(), MimePrepareError> {
        self.prepare_with_options(constraint, &MimePartWriteOptions::default())
    }

    /// Prepares the enclosed message without inspecting or changing the wrapper.
    ///
    /// An absent message is a no-op after options validation. A present message
    /// follows [`MimeMessage::prepare_with_options`], including MIME-Version
    /// insertion. Wrapper Content-Type and transfer encoding are not checked,
    /// repaired, or applied to the enclosed data. Writing validates them later.
    /// Traversal and partial-failure rules follow [`MimeEntity::prepare_with_options`].
    ///
    /// # Errors
    ///
    /// Options fail before mutation; enclosed-message failures retain prior edits.
    pub fn prepare_with_options(
        &mut self,
        constraint: EncodingConstraint,
        options: &MimePartWriteOptions,
    ) -> Result<(), MimePrepareError> {
        prepare_tree(Task::MessagePart(self), constraint, options)
    }
}

impl<R: Read + Seek> MimeMessage<R> {
    /// Prepares this message's body and MIME-Version using default write options.
    ///
    /// # Errors
    ///
    /// Returns [`MimePrepareError`], possibly after prior header edits.
    pub fn prepare(&mut self, constraint: EncodingConstraint) -> Result<(), MimePrepareError> {
        self.prepare_with_options(constraint, &MimePartWriteOptions::default())
    }

    /// Explicitly prepares this message and all attached message bodies.
    ///
    /// Adapts MimeKit's `MimeMessage.Prepare`. A bodyless message is a no-op
    /// after options validation, without parsing its MIME-Version. With a body,
    /// parses the first message-level MIME-Version and adds `MIME-Version: 1.0`
    /// only when absent and the body's root header list is nonempty. The check
    /// happens before descending; existing valid versions, including non-1.0
    /// values, preserve their raw bytes and duplicates. Misplaced body-level
    /// versions do not replace message-level metadata.
    ///
    /// No Date, Message-Id, addresses, or other message metadata is generated or
    /// refolded. Bcc and Resent-Bcc are not removed. Body/container rules follow
    /// [`MimeEntity::prepare_with_options`]; pass the same options when writing.
    /// Constructors, loading, and writing still never prepare implicitly.
    ///
    /// # Errors
    ///
    /// All options are validated before mutation. An invalid first MIME-Version
    /// is an explicit error, not MimeKit's missing-value fallback, even when the
    /// body's headers are empty. The body is not visited on that error. If a
    /// later descendant fails, a version already inserted here remains, along
    /// with any earlier leaf edits. This operation is not a tree transaction.
    ///
    /// ```
    /// use mimecraft::{EncodingConstraint, MimeMessage, MimeVersion, TextPart};
    ///
    /// let mut message = MimeMessage::with_body(TextPart::new("plain", "hello")?.into());
    /// assert_eq!(message.mime_version()?, None);
    /// message.prepare(EncodingConstraint::SevenBit)?;
    /// assert_eq!(message.mime_version()?, Some(MimeVersion::V1_0));
    /// let mut output = Vec::new();
    /// message.write_to(&mut output)?;
    /// assert!(output.starts_with(b"MIME-Version: 1.0\r\n"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn prepare_with_options(
        &mut self,
        constraint: EncodingConstraint,
        options: &MimePartWriteOptions,
    ) -> Result<(), MimePrepareError> {
        prepare_tree(Task::Message(self), constraint, options)
    }
}

enum Task<'a, R> {
    Entity(&'a mut MimeEntity<R>),
    Multipart(&'a mut Multipart<R>),
    MessagePart(&'a mut MessagePart<R>),
    Message(&'a mut MimeMessage<R>),
    Children(slice::IterMut<'a, MimeEntity<R>>),
}

fn prepare_tree<R: Read + Seek>(
    root: Task<'_, R>,
    constraint: EncodingConstraint,
    options: &MimePartWriteOptions,
) -> Result<(), MimePrepareError> {
    options
        .validate()
        .map_err(MimePrepareError::InvalidOptions)?;
    let mut stack = vec![root];
    while let Some(task) = stack.pop() {
        match task {
            Task::Entity(entity) => match entity {
                MimeEntity::Part(part) => part
                    .prepare_with_options(constraint, options)
                    .map_err(MimePrepareError::Part)?,
                MimeEntity::Multipart(multipart) => stack.push(Task::Multipart(multipart)),
                MimeEntity::Message(part) => stack.push(Task::MessagePart(part)),
            },
            Task::Multipart(multipart) => {
                // Keep a child iterator instead of pushing every sibling, so a
                // wide multipart does not enlarge the traversal stack.
                stack.push(Task::Children(multipart.children_mut().iter_mut()));
            }
            Task::Children(mut children) => {
                if let Some(child) = children.next() {
                    stack.push(Task::Children(children));
                    stack.push(Task::Entity(child));
                }
            }
            Task::MessagePart(part) => {
                if let Some(message) = part.message_mut() {
                    stack.push(Task::Message(message));
                }
            }
            Task::Message(message) => {
                if let Some(body) = message.body() {
                    // MimeKit inserts the version before preparing the body,
                    // but only when that body already has at least one header.
                    if message
                        .mime_version()
                        .map_err(MimePrepareError::MimeVersion)?
                        .is_none()
                        && !body.headers().is_empty()
                    {
                        message.set_mime_version(MimeVersion::V1_0);
                    }
                    if let Some(body) = message.body_mut() {
                        stack.push(Task::Entity(body));
                    }
                }
            }
        }
    }
    Ok(())
}

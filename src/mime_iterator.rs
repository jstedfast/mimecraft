// Adapted from MimeKit's MimeIterator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, fmt::Write as _};

use crate::{MimeEntity, MimeMessage};

/// Invalid syntax in a MIME part path; offsets are UTF-8 byte positions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MimePathParseError {
    /// A component is empty, including an empty path or a trailing dot.
    EmptyComponent {
        /// Position where a decimal component was expected.
        offset: usize,
    },
    /// A component contains something other than ASCII decimal digits.
    InvalidCharacter {
        /// Position of the first non-digit byte within a component.
        offset: usize,
    },
    /// A component exceeds MimeKit's signed 32-bit bound.
    ComponentOverflow {
        /// Position of the digit that exceeds the component bound.
        offset: usize,
    },
}

impl fmt::Display for MimePathParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyComponent { offset } => {
                write!(f, "empty MIME path component at byte {offset}")
            }
            Self::InvalidCharacter { offset } => {
                write!(f, "invalid MIME path character at byte {offset}")
            }
            Self::ComponentOverflow { offset } => {
                write!(f, "MIME path component exceeds i32::MAX at byte {offset}")
            }
        }
    }
}

impl Error for MimePathParseError {}

/// An iterator operation requires a current entity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MimeIteratorPositionError;

impl fmt::Display for MimeIteratorPositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the MIME iterator has no current entity")
    }
}

impl Error for MimeIteratorPositionError {}

// Each frame is one level of the path from the message body to the current
// entity. Store the one-based part number, using zero for multipart message
// bodies: they have no IMAP part number of their own.
struct Frame<'a, R> {
    entity: &'a MimeEntity<R>,
    number: usize,
}

impl<'a, R> Frame<'a, R> {
    fn message_body(entity: &'a MimeEntity<R>) -> Self {
        Self {
            entity,
            number: usize::from(!matches!(entity, MimeEntity::Multipart(_))),
        }
    }
}

/// A borrowed, depth-first iterator over every entity in a message.
///
/// Returns multipart containers and attached-message wrappers as well as leaves,
/// descending into attached message bodies. Unlike [`crate::BodyParts`], this
/// visits the complete tree. Traversal follows enum variants, not parsed headers,
/// and requires no source I/O or `Read`/`Seek` bounds.
///
/// Maintains an iterative, depth-proportional frame stack. Metadata describes
/// the most recently returned entity. Before the first item and after exhaustion,
/// [`Self::current`] and other position queries return `None`. Exhaustion persists
/// until [`Self::reset`] or successful [`Self::move_to`].
///
/// The tree is immutably borrowed; returned entities remain valid while the
/// iterator advances. Tree mutation during traversal is not supported.
///
/// ```
/// use mimecraft::{MessagePart, MimeEntity, MimeMessage, TextPart};
///
/// let inner = MimeMessage::with_body(TextPart::new("plain", "hello")?.into());
/// let message = MimeMessage::with_body(MessagePart::new(inner).into());
/// let mut iter = message.iter();
/// while let Some(entity) = iter.next() {
///     let path = iter.path_specifier().unwrap();
///     assert!(std::ptr::eq(message.entity_at(&path)?.unwrap(), entity));
///     if matches!(entity, MimeEntity::Message(_)) {
///         iter.skip_children()?;
///     }
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct MimeIterator<'a, R> {
    message: &'a MimeMessage<R>,
    frames: Vec<Frame<'a, R>>,
    move_first: bool,
    skip_children: bool,
}

impl<'a, R> MimeIterator<'a, R> {
    /// Starts before the body of the supplied message, without allocating.
    pub fn new(message: &'a MimeMessage<R>) -> Self {
        Self {
            message,
            frames: Vec::new(),
            move_first: true,
            skip_children: false,
        }
    }

    /// Returns the top-level message, regardless of iterator position.
    pub fn message(&self) -> &'a MimeMessage<R> {
        self.message
    }

    /// Returns the current entity, or `None` before iteration or after exhaustion.
    pub fn current(&self) -> Option<&'a MimeEntity<R>> {
        self.frames.last().map(|frame| frame.entity)
    }

    /// Returns the current entity's parent, or `None` at the root or without a position.
    ///
    /// An attached message body's parent is its message wrapper.
    pub fn parent(&self) -> Option<&'a MimeEntity<R>> {
        self.ancestor(1)
    }

    /// Returns an ancestor, where one generation means the parent.
    ///
    /// Returns `None` without a position, for zero, or beyond the current depth.
    pub fn ancestor(&self, generation: usize) -> Option<&'a MimeEntity<R>> {
        if generation == 0 {
            return None;
        }
        let index = self.frames.len().checked_sub(1)?.checked_sub(generation)?;
        Some(self.frames[index].entity)
    }

    /// Returns the tree depth, including message-body frames; the root has depth zero.
    pub fn depth(&self) -> Option<usize> {
        self.frames.len().checked_sub(1)
    }

    /// Returns the zero-based index within a multipart parent.
    ///
    /// Message bodies and an unpositioned iterator return `None`.
    pub fn index(&self) -> Option<usize> {
        match self.parent()? {
            MimeEntity::Multipart(_) => self.frames.last()?.number.checked_sub(1),
            _ => None,
        }
    }

    /// Returns the message containing the current entity, or `None` without a position.
    ///
    /// A message wrapper belongs to the outer message; its body and descendants
    /// belong to the wrapped message.
    pub fn current_message(&self) -> Option<&'a MimeMessage<R>> {
        let last = self.frames.len().checked_sub(1)?;
        for frame in self.frames[..last].iter().rev() {
            if let MimeEntity::Message(part) = frame.entity
                && let Some(message) = part.message()
            {
                return Some(message);
            }
        }
        Some(self.message)
    }

    /// Formats the current part path, or returns `None` without a position.
    ///
    /// Multipart children are numbered from one. A non-multipart message body
    /// is part `1`. MimeKit's `0` extension identifies multipart message bodies:
    /// `0` for a top-level multipart, or `3.0` for the multipart body inside
    /// message wrapper `3`. Its children are `3.1`, `3.2`, etc., not `3.0.1`.
    ///
    /// Allocates a new string; traversal itself does not format paths.
    pub fn path_specifier(&self) -> Option<String> {
        let (last, ancestors) = self.frames.split_last()?;
        let mut path = String::new();
        for frame in ancestors {
            // Multipart message bodies have no part number of their own.
            if frame.number != 0 {
                write!(path, "{}.", frame.number).expect("writing to a String cannot fail");
            }
        }
        write!(path, "{}", last.number).expect("writing to a String cannot fail");
        Some(path)
    }

    /// Skips the current entity's descendants on the next call to `next`.
    ///
    /// On a leaf this has no effect. Resetting or successfully parsing a new
    /// navigation request cancels the pending skip.
    ///
    /// # Errors
    ///
    /// Returns [`MimeIteratorPositionError`] before iteration or after exhaustion.
    pub fn skip_children(&mut self) -> Result<(), MimeIteratorPositionError> {
        self.current().ok_or(MimeIteratorPositionError)?;
        self.skip_children = true;
        Ok(())
    }

    /// Returns to the initial, before-first position and cancels pending skips.
    pub fn reset(&mut self) {
        self.frames.clear();
        self.move_first = true;
        self.skip_children = false;
    }

    /// Navigates directly to a part path, without visiting unrelated siblings.
    ///
    /// Accepts dot-separated ASCII decimal components in `0..=i32::MAX`,
    /// including leading zeros. A zero can only be the final component and
    /// refer to a multipart message body. Numbering follows [`Self::path_specifier`].
    ///
    /// Returns `Ok(true)` at the requested entity; `next` then visits its
    /// successor. A valid but nonexistent path returns `Ok(false)` and exhausts
    /// the iterator. Both outcomes cancel pending skips. Can navigate backwards
    /// or restart after exhaustion. Cost is proportional to path length and
    /// traversed depth, not the number of siblings.
    ///
    /// # Errors
    ///
    /// Returns [`MimePathParseError`] for invalid syntax or overflow, leaving the
    /// current position and pending skip unchanged. The whole path is checked
    /// before navigating, including components beyond nonexistent entities.
    pub fn move_to(&mut self, path: &str) -> Result<bool, MimePathParseError> {
        let numbers = parse_path(path)?;
        self.frames.clear();
        self.move_first = false;
        self.skip_children = false;
        if self.navigate(&numbers) {
            Ok(true)
        } else {
            self.frames.clear();
            Ok(false)
        }
    }

    fn navigate(&mut self, numbers: &[usize]) -> bool {
        let Some(body) = self.message.body() else {
            return false;
        };
        self.frames.push(Frame::message_body(body));
        let mut i = 0;
        if !matches!(body, MimeEntity::Multipart(_)) {
            // A non-multipart top-level body is part 1.
            if numbers[0] != 1 {
                return false;
            }
            i += 1;
        }
        while i < numbers.len() {
            let frame = self.frames.last().expect("navigation starts at the body");
            let number = numbers[i];
            match frame.entity {
                MimeEntity::Multipart(multipart) => {
                    if number == 0 {
                        // Zero refers only to a multipart message body and must
                        // be the last component, not a prefix for its children.
                        return frame.number == 0 && i == numbers.len() - 1;
                    }
                    let Some(entity) = multipart.children().get(number - 1) else {
                        return false;
                    };
                    self.frames.push(Frame { entity, number });
                    i += 1;
                }
                MimeEntity::Message(part) => {
                    let Some(body) = part.message().and_then(MimeMessage::body) else {
                        return false;
                    };
                    if !matches!(body, MimeEntity::Multipart(_)) {
                        if number != 1 {
                            return false;
                        }
                        i += 1;
                    }
                    // Multipart message bodies have no number: reuse this path
                    // component for the body itself (zero) or one of its children.
                    self.frames.push(Frame::message_body(body));
                }
                MimeEntity::Part(_) => return false,
            }
        }
        true
    }
}

impl<'a, R> Iterator for MimeIterator<'a, R> {
    type Item = &'a MimeEntity<R>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.move_first {
            self.move_first = false;
            let body = self.message.body()?;
            self.frames.push(Frame::message_body(body));
            return Some(body);
        }
        let current = self.current()?;
        if self.skip_children {
            self.skip_children = false;
        } else {
            match current {
                MimeEntity::Message(part) => {
                    if let Some(body) = part.message().and_then(MimeMessage::body) {
                        self.frames.push(Frame::message_body(body));
                        return Some(body);
                    }
                }
                MimeEntity::Multipart(multipart) => {
                    if let Some(entity) = multipart.children().first() {
                        self.frames.push(Frame { entity, number: 1 });
                        return Some(entity);
                    }
                }
                MimeEntity::Part(_) => {}
            }
        }
        // Find the next sibling of the current entity, or of its nearest
        // ancestor that has one, without recursion or flattening the tree.
        while self.frames.len() > 1 {
            let frame = self.frames.pop().expect("the current frame exists");
            if let MimeEntity::Multipart(parent) = self.frames.last()?.entity
                && let Some(entity) = parent.children().get(frame.number)
            {
                self.frames.push(Frame {
                    entity,
                    number: frame.number + 1,
                });
                return Some(entity);
            }
        }
        self.frames.clear();
        None
    }
}

fn parse_path(path: &str) -> Result<Vec<usize>, MimePathParseError> {
    let mut numbers = Vec::new();
    let mut offset = 0;
    for component in path.split('.') {
        if component.is_empty() {
            return Err(MimePathParseError::EmptyComponent { offset });
        }
        let mut number = 0usize;
        for byte in component.bytes() {
            if !byte.is_ascii_digit() {
                return Err(MimePathParseError::InvalidCharacter { offset });
            }
            let digit = usize::from(byte - b'0');
            if number > (i32::MAX as usize - digit) / 10 {
                return Err(MimePathParseError::ComponentOverflow { offset });
            }
            number = number * 10 + digit;
            offset += 1;
        }
        numbers.push(number);
        offset += 1;
    }
    Ok(numbers)
}

// Adapted from MimeKit's MimeReader.cs and MimeParser.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead};

use crate::{MimeEntity, MimeEntityParseOptions, MimeParser};

impl MimeEntity {
    /// Loads one MIME entity through EOF into owned in-memory body streams.
    ///
    /// Uses default [`MimeEntityParseOptions`]. For plain [`io::Read`] sources,
    /// wrap the source in [`io::BufReader`]. No seeking or source closing occurs.
    /// See [`Self::load_with_options`] for structural rules and limitations.
    ///
    /// # Errors
    ///
    /// Returns invalid-input, resource-limit, or I/O errors without a partial tree.
    ///
    /// ```
    /// use mimecraft::{MimeEntity, TextPart};
    ///
    /// let mut input = &b"Content-Type: text/plain; charset=utf-8\r\n\r\nhello"[..];
    /// let MimeEntity::Part(part) = MimeEntity::load(&mut input)? else {
    ///     unreachable!()
    /// };
    /// assert_eq!(TextPart::from_part(part)?.text()?, "hello");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn load<R: BufRead + ?Sized>(reader: &mut R) -> io::Result<Self> {
        Self::load_with_options(reader, &MimeEntityParseOptions::default())
    }

    /// Loads a complete entity with explicit limits and message interpretation.
    ///
    /// Uses [`MimeParser`] over the streaming [`crate::MimeReader`], retaining
    /// headers, preambles, epilogues, and encoded leaf bodies in memory. Use
    /// the reader directly for events without constructing a complete tree.
    /// This owned loader is not constant-memory, zero-copy, or persistent-source.
    /// The resulting source type is [`io::Cursor<Vec<u8>>`], positioned at zero.
    /// Body encoding comes from the first Content-Transfer-Encoding header;
    /// missing means `Unspecified`. Body bytes are not decoded or newline-normalized.
    /// Newline hints describe stored LF/CRLF occurrences, excluding framing.
    ///
    /// Raw headers retain spelling, folds, duplicates, and malformed records
    /// using [`crate::HeaderParser`]. Missing Content-Type selects an opaque leaf,
    /// except that immediate multipart/digest children default to message/rfc822.
    /// That default does not extend into the embedded body or other containers.
    /// No missing header is synthesized. Invalid Content-Type falls back to
    /// application/octet-stream; recognizable types survive broken parameters.
    /// Unknown transfer encodings use `Unspecified`. The original metadata is
    /// retained, so explicit typed-header getters can still report syntax errors.
    /// Use a compliance logger to inspect or reject malformed metadata.
    /// `multipart/*` creates a
    /// container. Recognized, transparently encoded message parts create
    /// [`crate::MessagePart`] nodes unless `parse_message_parts` is false; see
    /// [`MimeEntityParseOptions::parse_message_parts`] for supported types.
    /// Each embedded message partitions its own root Content-* headers onto
    /// its body. Limits are shared across the entire tree, not reset per message.
    /// Encoded wrappers and other types remain opaque leaves.
    ///
    /// Delimiters match whole, case-sensitive lines with optional trailing
    /// space/tab. CRLF and LF are accepted, as is a closing marker at EOF.
    /// Preambles retain their ending newline. Leaf content and nested epilogues
    /// exclude exactly the LF or CRLF immediately preceding a parent delimiter;
    /// at EOF all trailing content bytes are retained. Epilogues begin after
    /// the closing marker's line ending, matching the [`crate::Multipart`] model.
    ///
    /// By default, headers interrupted by EOF or an enclosing delimiter complete
    /// without a separator. Missing closing markers unwind truncated containers;
    /// missing boundary parameters leave all content in the preamble. Invalid
    /// boundaries are matched literally, nearest ancestor first. Unchanged parsed
    /// metadata retains this recovery interpretation when writing; authored or
    /// edited metadata must pass writer validation.
    /// A message wrapper immediately followed by an enclosing marker has no
    /// attached message; standalone EOF constructs an empty embedded message.
    /// Empty top-level input and an invalid first header are still errors.
    /// Parsed markers, truncation, separator presence, and no-content newline
    /// ownership are retained for serialization. Headers and nonbinary content
    /// still follow output-newline options, so this is not a raw-wire API.
    ///
    /// # Errors
    ///
    /// Zero-valued limits return [`io::ErrorKind::InvalidInput`] before I/O.
    /// Invalid initial headers and exceeded limits return
    /// [`io::ErrorKind::InvalidData`]; empty input or explicitly rejected
    /// header-only EOF returns [`io::ErrorKind::UnexpectedEof`]. Header-length errors retain
    /// [`crate::HeaderTooLong`] as their cause. Other I/O errors propagate and
    /// interrupted reads retry. Source position is not restored; retrying needs
    /// a fresh or repositioned source. Reader read-ahead can exceed consumed
    /// bytes; retain the same buffered reader after an error.
    pub fn load_with_options<R: BufRead + ?Sized>(
        reader: &mut R,
        options: &MimeEntityParseOptions,
    ) -> io::Result<Self> {
        MimeParser::with_options(reader, *options)?.parse_entity()
    }
}

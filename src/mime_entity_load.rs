// Adapted from MimeKit's MimeEntity.cs, MimeReader.cs and MimeParser.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Read, Seek};
use std::path::Path;

use crate::{
    ContentType, MimeContentStorage, MimeContentStream, MimeEntity, MimeEntityParseOptions,
    MimeParser,
};

impl MimeEntity {
    /// Loads one complete entity from a file into owned in-memory body streams.
    ///
    /// Uses default [`MimeEntityParseOptions`]. The file is opened read-only and
    /// closed before returning, including on failure. See
    /// [`Self::load_file_with_options`] for parsing and error behavior.
    ///
    /// # Errors
    ///
    /// Propagates file-open, read, and parsing errors without a partial entity.
    pub fn load_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::load_file_with_options(path, &MimeEntityParseOptions::default())
    }

    /// Loads an entity from a file with explicit parsing options.
    ///
    /// Opens the file read-only and parses through EOF with
    /// [`Self::load_with_options`]. Encoded bodies are copied into memory;
    /// neither the entity nor its children retain a file handle. Header offsets
    /// are relative to the beginning of the file. Recovery and limits are the
    /// same as for buffered-stream loading.
    ///
    /// For source-backed content instead, open a [`File`] and pass ownership to
    /// [`Self::load_with_storage`] with [`MimeContentStorage::Source`].
    ///
    /// # Errors
    ///
    /// Validates options before opening the file. Platform path and access
    /// errors, read errors, and parsing errors propagate unchanged. The file is
    /// closed on success or failure; no partial entity is returned.
    ///
    /// ```no_run
    /// use mimecraft::{MimeEntity, MimeEntityParseOptions};
    ///
    /// let options = MimeEntityParseOptions {
    ///     max_input_bytes: 1024 * 1024,
    ///     ..Default::default()
    /// };
    /// let entity = MimeEntity::load_file_with_options("entity.mime", &options)?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn load_file_with_options<P: AsRef<Path>>(
        path: P,
        options: &MimeEntityParseOptions,
    ) -> io::Result<Self> {
        options.validate()?;
        let mut reader = BufReader::new(File::open(path)?);
        Self::load_with_options(&mut reader, options)
    }

    /// Loads a body whose Content-Type was parsed separately from its bytes.
    ///
    /// Intended for APIs such as HTTP clients that expose headers separately
    /// from the content stream. Prepends a formatted Content-Type and a blank
    /// line, then uses the ordinary copying entity loader. The supplied body
    /// does not need MIME headers; header-like leading bytes remain content.
    ///
    /// Multipart bodies and transparent message wrappers are parsed normally.
    /// No transfer encoding is inferred or supplied for the root. Copied leaf
    /// bytes remain undecoded, including transfer-encoded multipart children.
    /// The returned model does not borrow the content or the Content-Type.
    /// The content stream is neither sought nor closed. Wrap plain [`Read`]
    /// sources in [`io::BufReader`]; nonseekable [`BufRead`] sources work directly.
    ///
    /// # Errors
    ///
    /// Uses default parse limits. Formatting or parsing errors return no partial
    /// model; see [`Self::load_content_with_options`] for limits and offsets.
    ///
    /// ```
    /// use mimecraft::{ContentType, MimeEntity, TextPart};
    ///
    /// let content_type = ContentType::parse(b"text/plain; charset=utf-8")?;
    /// let mut content = b"This is the response body.".as_slice();
    /// let MimeEntity::Part(part) = MimeEntity::load_content(&content_type, &mut content)? else {
    ///     unreachable!()
    /// };
    /// assert_eq!(TextPart::from_part(part)?.text()?, "This is the response body.");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn load_content<R: BufRead + ?Sized>(
        content_type: &ContentType,
        content: &mut R,
    ) -> io::Result<Self> {
        Self::load_content_with_options(content_type, content, &MimeEntityParseOptions::default())
    }

    /// Loads a body with a separately supplied Content-Type and explicit limits.
    ///
    /// Follows [`Self::load_content`], with options applied to the entire
    /// synthesized MIME entity. Input-byte and header-length budgets include
    /// the formatted Content-Type; the input budget also includes its blank
    /// separator. Header offsets include this prefix, not just the body bytes.
    /// Nested-message interpretation and recovery follow [`Self::load_with_options`].
    ///
    /// The prefix uses the existing Content-Type formatter's default folding
    /// and RFC 2231 parameter encoding, with CRLF termination and a CRLF blank
    /// line. This does not modify the supplied Content-Type or normalize body
    /// bytes. Only the prefix is buffered before parsing begins.
    ///
    /// # Errors
    ///
    /// Invalid options fail before formatting or source I/O. An unformattable
    /// Content-Type returns [`io::ErrorKind::InvalidInput`] containing
    /// [`crate::HeaderFormatError`], also before I/O. Parse and source errors
    /// propagate unchanged. On failure, the caller's source remains open but
    /// consumed bytes are not restored; buffered read-ahead stays with it.
    pub fn load_content_with_options<R: BufRead + ?Sized>(
        content_type: &ContentType,
        content: &mut R,
        options: &MimeEntityParseOptions,
    ) -> io::Result<Self> {
        options.validate()?;
        let mut header = content_type
            .to_header_string()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        header.push_str("\r\n");
        // Like MimeKit's ChainedStream, prepend MIME headers without copying
        // the content stream. Borrowing replaces its leaveOpen ownership flag.
        let mut input = Cursor::new(header.into_bytes()).chain(content);
        Self::load_with_options(&mut input, options)
    }

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

    /// Loads a complete entity with explicit encoded-body storage.
    ///
    /// [`MimeContentStorage::Memory`] copies bodies; [`MimeContentStorage::Source`]
    /// retains ranges in the seekable input. Headers and framing are stored in
    /// memory in both modes. Parsing and recovery follow [`Self::load_with_options`].
    ///
    /// Takes an owned source or a mutable borrow at its current position.
    /// Owned sources stay alive as long as source-backed content needs them;
    /// dropping an entity never closes a borrowed source. Keep the underlying
    /// data unchanged while source-backed content is in use.
    ///
    /// Both choices require `Read + Seek` and return [`MimeContentStream<R>`].
    /// That type retains `R`'s lifetime even in `Memory` mode. For nonseekable
    /// input, or copies independent of a borrowed source's lifetime, use
    /// [`Self::load`] instead. There is no nonseekable fallback.
    ///
    /// # Errors
    ///
    /// Propagates initial source-position and parsing errors without a partial
    /// entity or position rollback. Uses default limits; see
    /// [`Self::load_with_storage_and_options`] for explicit limits.
    ///
    /// ```
    /// use std::io::Cursor;
    /// use mimecraft::{MimeContentStorage, MimeEntity};
    ///
    /// let input = Cursor::new(b"Content-Transfer-Encoding: base64\r\n\r\naGVsbG8=");
    /// let MimeEntity::Part(mut part) =
    ///     MimeEntity::load_with_storage(input, MimeContentStorage::Source)? else {
    ///     unreachable!()
    /// };
    /// let mut decoded = Vec::new();
    /// part.content_mut().decode_to(&mut decoded)?;
    /// assert_eq!(decoded, b"hello");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// A borrowed entity cannot outlive the backing source:
    ///
    /// ```compile_fail
    /// use std::io::Cursor;
    /// use mimecraft::{MimeContentStorage, MimeEntity};
    ///
    /// let mut entity = {
    ///     let mut source = Cursor::new(b"\r\nbody");
    ///     MimeEntity::load_with_storage(&mut source, MimeContentStorage::Source).unwrap()
    /// };
    /// entity.write_to(&mut Vec::new()).unwrap();
    /// ```
    ///
    /// Source storage cannot silently copy nonseekable input:
    ///
    /// ```compile_fail
    /// use mimecraft::{MimeContentStorage, MimeEntity};
    ///
    /// let entity = MimeEntity::load_with_storage(b"\r\nbody".as_slice(), MimeContentStorage::Source);
    /// ```
    pub fn load_with_storage<R: Read + Seek>(
        source: R,
        storage: MimeContentStorage,
    ) -> io::Result<MimeEntity<MimeContentStream<R>>> {
        Self::load_with_storage_and_options(source, storage, &MimeEntityParseOptions::default())
    }

    /// Loads an entity with explicit body storage and parsing options.
    ///
    /// Ownership and storage follow [`Self::load_with_storage`]. Uses the same
    /// [`MimeParser`] path as the copying loader, including nested messages,
    /// malformed-message recovery, and delimiter-newline ownership.
    ///
    /// # Errors
    ///
    /// Validates options before any source I/O. Then queries the initial source
    /// position and parses through EOF, propagating all errors unchanged.
    /// On failure, an owned source is dropped; a borrowed source stays open
    /// but is not rewound. Internal read-ahead is discarded with the parser.
    /// Use [`MimeParser`] directly to recover its buffered source after errors
    /// or to parse an mbox archive incrementally.
    pub fn load_with_storage_and_options<R: Read + Seek>(
        source: R,
        storage: MimeContentStorage,
        options: &MimeEntityParseOptions,
    ) -> io::Result<MimeEntity<MimeContentStream<R>>> {
        MimeParser::with_storage_and_options(source, storage, *options)?.parse_entity()
    }
}

// Adapted from MimeKit's MimeReader.cs and MimeParser.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io;

/// Resource limits and message interpretation for MIME reading and parsing.
///
/// All limits must be positive. These are not transport constraints and do
/// not limit decoded payload size; transfer decoding happens only on request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MimeEntityParseOptions {
    /// Maximum consumed input bytes, including all framing. Defaults to 64 MiB.
    ///
    /// This bounds wire data, not total allocation overhead or reader read-ahead.
    /// Standalone header reading counts headers and their separator, not the body.
    /// Status-group iteration shares this budget across groups and blank lines.
    /// Mbox iteration resets it per message, including its envelope marker and
    /// initial skipped data, but excluding next-marker lookahead.
    pub max_input_bytes: usize,
    /// Maximum entity depth, counting the root as one. Defaults to 64.
    ///
    /// Message-part wrappers and embedded body roots each count as entities.
    /// Loading is iterative, but model debugging/drop recurse. Choose a safe
    /// bound for your application rather than disabling this protection.
    /// Standalone header reading does not construct entities or apply this limit.
    pub max_depth: usize,
    /// Maximum raw bytes per header record, including folds. Defaults to 16 KiB.
    ///
    /// Also bounds each complete mbox marker line, including its LF or CRLF.
    pub max_header_length: usize,
    /// Parse recognized, transparently encoded message parts structurally.
    ///
    /// Defaults to true. Set false to retain all message parts as opaque leaves.
    /// Recognizes message/rfc822, news, global, global-headers, external-body,
    /// and rfc2822, plus text/rfc822-headers and immediate multipart/digest
    /// children without Content-Type (implicitly message/rfc822).
    /// No missing header is synthesized.
    /// Base64, quoted-printable, and UU wrappers remain opaque regardless of
    /// this option. Truncated inner headers recover like ordinary entity headers.
    /// Existing bodies are not discarded.
    pub parse_message_parts: bool,
    /// Honor the outer message's Content-Length when reading an mbox archive.
    ///
    /// Defaults to false. When enabled, `From ` lines before the declared body
    /// end are ordinary data, including inside embedded messages. MIME
    /// boundaries still match normally. The length is a minimum marker offset,
    /// not a hard body limit: reading continues to the next eligible marker or
    /// EOF. Input limits still apply, and a length beyond EOF is not an error.
    ///
    /// Matches MimeKit's first usable raw Content-Length: leading MIME
    /// whitespace is required, followed by unsigned decimal digits fitting in
    /// `i32`; trailing text is ignored. Invalid values are skipped. Nested
    /// Content-Length fields do not override the outer length. Standalone
    /// entity/message/header reading is unaffected.
    pub respect_content_length: bool,
    /// Accept entity/message headers interrupted by EOF or an enclosing marker.
    ///
    /// Defaults to true, matching MimeKit's recovery. Applies to embedded headers
    /// and empty multipart children too. Unterminated records remain raw; no
    /// separator bytes or metadata headers are synthesized during reading.
    ///
    /// Retained for compatibility; false rejects missing separators but is not
    /// a general strict mode. Prefer a fallible compliance logger to reject
    /// selected violations. Empty input and resource/source errors remain errors.
    /// Recovery never guesses where body text starts. Serialization adds normal
    /// framing; missing separators are not stored in the model.
    /// Standalone header and status-group parsing already accept EOF.
    pub allow_header_only: bool,
}

impl Default for MimeEntityParseOptions {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_depth: 64,
            max_header_length: 16 * 1024,
            parse_message_parts: true,
            respect_content_length: false,
            allow_header_only: true,
        }
    }
}

impl MimeEntityParseOptions {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.max_input_bytes == 0 || self.max_depth == 0 || self.max_header_length == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MIME parser limits must be positive",
            ));
        }
        Ok(())
    }
}

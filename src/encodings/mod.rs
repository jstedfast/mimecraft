// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! MIME transfer encodings and header payload codecs.
//!
//! Codecs append to caller-owned byte buffers, which can be drained or cleared
//! between calls. Incremental codecs preserve state across input chunks;
//! cloning copies that state and resetting discards pending input. The RFC 2047
//! encoders instead write complete payloads per call, without wrappers or folding.
//! [`MimeEncoder`] and [`MimeDecoder`] provide common incremental interfaces;
//! pass-through codecs copy bytes unchanged with configurable encoding metadata.

mod base64;
mod hex;
mod pass_through;
mod quoted_printable;
mod rfc2047;
mod rfc2231;
mod traits;
mod uu;
mod yenc;

use std::{error::Error, fmt};

pub use base64::{Base64Decoder, Base64Encoder};
pub use pass_through::{PassThroughDecoder, PassThroughEncoder};
pub use quoted_printable::{QuotedPrintableDecoder, QuotedPrintableEncoder};
pub use rfc2047::{QEncodeMode, Rfc2047Base64Encoder, Rfc2047QuotedPrintableEncoder};
pub use rfc2231::{Rfc2231Decoder, Rfc2231Encoder};
pub use traits::{MimeDecoder, MimeEncoder};
pub use uu::{UuDecoder, UuEncoder};
pub use yenc::{YDecoder, YEncoder};

/// A new-line format, matching MimeKit's `NewLineFormat`.
///
/// DOS (CRLF) is the default. Encoders accept DOS or Unix (LF), but not mixed
/// line endings. Line-length limits exclude newline bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NewLineFormat {
    /// Unix line endings: line feed only (`\n`).
    Unix,
    /// DOS line endings: carriage return followed by line feed (`\r\n`).
    #[default]
    Dos,
    /// A mixture of Unix and DOS line endings; not a valid encoder output format.
    Mixed,
}

impl NewLineFormat {
    fn as_bytes(self) -> &'static [u8] {
        match self {
            Self::Dos => b"\r\n",
            Self::Unix => b"\n",
            // Encoder constructors reject Mixed before any output can be emitted.
            Self::Mixed => unreachable!("mixed new-line format is not valid for encoders"),
        }
    }
}

/// A requested line length outside the supported range of 60..=998.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidLineLength {
    requested: usize,
}

impl InvalidLineLength {
    pub(crate) fn new(requested: usize) -> Self {
        Self { requested }
    }

    /// Returns the rejected line length.
    pub fn requested(self) -> usize {
        self.requested
    }
}

impl fmt::Display for InvalidLineLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line length must be between 60 and 998 bytes, got {}",
            self.requested
        )
    }
}

impl Error for InvalidLineLength {}

/// Invalid options supplied when constructing a transfer encoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncoderOptionsError {
    /// The requested line length is outside 60..=998.
    InvalidLineLength(InvalidLineLength),
    /// The requested new-line format is not valid for encoder output.
    InvalidNewLineFormat(NewLineFormat),
}

impl fmt::Display for EncoderOptionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineLength(error) => fmt::Display::fmt(error, f),
            Self::InvalidNewLineFormat(format) => {
                write!(f, "new-line format must be Dos or Unix, got {format:?}")
            }
        }
    }
}

impl Error for EncoderOptionsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidLineLength(error) => Some(error),
            Self::InvalidNewLineFormat(_) => None,
        }
    }
}

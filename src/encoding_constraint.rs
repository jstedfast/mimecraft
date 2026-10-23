// Ported from MimeKit's EncodingConstraint.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

/// A transport constraint used when choosing a content transfer encoding.
///
/// Not every transport supports binary or eight-bit data. This describes what
/// the transport permits; it does not itself validate or transform content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EncodingConstraint {
    /// Allow any byte, including NUL, in unencoded content.
    #[default]
    None,
    /// Allow high-bit bytes in unencoded content, but not NUL.
    EightBit,
    /// Restrict the transport to seven-bit data.
    SevenBit,
}

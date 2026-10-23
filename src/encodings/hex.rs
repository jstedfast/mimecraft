// Ported from MimeKit.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

const HEX: &[u8; 16] = b"0123456789ABCDEF";

pub(super) fn encode_hex_byte(prefix: u8, byte: u8, output: &mut Vec<u8>) {
    output.extend_from_slice(&[prefix, HEX[(byte >> 4) as usize], HEX[(byte & 15) as usize]]);
}

pub(super) fn hex_rank(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

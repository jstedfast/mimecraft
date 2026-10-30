// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

pub fn fingerprint(bytes: &[u8]) -> (usize, u64) {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    (bytes.len(), hash)
}

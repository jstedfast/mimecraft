// Adapted from MimeKit's Utils/Crc32.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

// Generate MimeKit's reflected CRC-32 table at compile time. Callers choose
// their initial state and final complement: yEnc starts at all ones, whereas
// compressed RTF starts at zero and does not complement the result.
static TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    while index < table.len() {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};

pub(crate) fn update(crc: u32, byte: u8) -> u32 {
    TABLE[((crc ^ u32::from(byte)) & 255) as usize] ^ (crc >> 8)
}

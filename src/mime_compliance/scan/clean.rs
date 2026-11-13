// Adapted from MimeKit's MimeReader.SkipValidatedLines routines.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::mime_compliance) enum Validation {
    Structural,
    Base64,
    QuotedPrintable,
    Uu(i16),
}

#[derive(Default, Debug)]
pub(in crate::mime_compliance) struct Clean {
    pub len: usize,
    pub lines: usize,
    pub alphabet: usize,
    // Fully accepted blocks may also establish a newline-free tail. Reuse
    // that progress in the fallback instead of scanning those bytes again.
    pub examined: usize,
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
pub(super) struct Run {
    pub clean: Clean,
    column: u64,
    cr: bool,
    alphabet: usize,
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
impl Run {
    pub fn new(column: u64, cr: bool) -> Self {
        Self {
            clean: Clean::default(),
            column,
            cr,
            alphabet: 0,
        }
    }

    #[inline(always)]
    pub fn push(&mut self, input: &[u8], width: usize, lf: u32, cr: u32, alphabet: u32) -> bool {
        let offset = self.clean.examined;
        if lf != 0 {
            let carry = u32::from(if offset == 0 {
                self.cr
            } else {
                input[offset - 1] == b'\r'
            });
            // Only clean, complete CRLF lines are committed. Exceptional lines
            // go through the per-line scanner with its exact diagnostic order.
            if lf & !((cr << 1) | carry) != 0
                || self.column - 1
                    + (offset + lf.trailing_zeros() as usize + 1 - self.clean.len) as u64
                    > 1000
            {
                return false;
            }
            let last = 31 - lf.leading_zeros();
            self.clean.lines += lf.count_ones() as usize;
            self.clean.len = offset + last as usize + 1;
            self.clean.alphabet =
                self.alphabet + (alphabet & (u32::MAX >> (31 - last))).count_ones() as usize;
            self.column = 1;
        } else if self.column - 1 + (offset + width - self.clean.len) as u64 > 1000 {
            return false;
        }
        self.alphabet += alphabet.count_ones() as usize;
        self.clean.examined += width;
        true
    }
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
pub(super) fn escapes_valid(
    input: &[u8],
    offset: usize,
    width: usize,
    equals: u32,
    hex: u32,
    cr: u32,
    lf: u32,
) -> bool {
    let mut tail = equals & (3 << (width - 2));
    let valid = ((hex >> 1) & (hex >> 2)) | ((cr >> 1) & (lf >> 2));
    if equals & !tail & !valid != 0 {
        return false;
    }
    // Escapes crossing a vector edge require real input, never an LF sentinel.
    while tail != 0 {
        let escape = offset + tail.trailing_zeros() as usize;
        if input.len() - escape < 3
            || !(input[escape + 1..escape + 3] == *b"\r\n"
                || (input[escape + 1].is_ascii_hexdigit() && input[escape + 2].is_ascii_hexdigit()))
        {
            return false;
        }
        tail &= tail - 1;
    }
    true
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
pub(super) fn uu(input: &[u8], mut remaining: i16, mut column: u64, mut cr: bool) -> Clean {
    let mut clean = Clean::default();
    while clean.len < input.len() {
        let mut payload = clean.len;
        let count = if remaining < 0 {
            let decoded = input[payload].wrapping_sub(32);
            // Leave wrappers and the zero-length terminator to the usual path.
            if !(1..=45).contains(&decoded) {
                break;
            }
            payload += 1;
            4 * usize::from(decoded).div_ceil(3)
        } else {
            remaining as usize
        };
        let end = payload + count;
        let newline = if remaining == 0 && cr { 1 } else { 2 };
        if end + newline > input.len()
            || input[end + newline - 1] != b'\n'
            || (newline == 2 && input[end] != b'\r')
            || column - 1 + (end + newline - clean.len) as u64 > 1000
            || !input[payload..end]
                .iter()
                .all(|byte| (33..=96).contains(byte))
        {
            break;
        }
        clean.len = end + newline;
        clean.lines += 1;
        remaining = -1;
        cr = false;
        column = 1;
    }
    clean.examined = clean.len;
    clean
}

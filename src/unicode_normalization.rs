// Adapted from MimeKit's Utils/UnicodeNormalization.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use crate::smtp_utf8_data::{COMPOSITION_PAIRS, COMPOSITIONS, NORMALIZATION};

// The Unicode Standard, Section 3.12: Conjoining Jamo Behavior.
const S_BASE: u32 = 0xac00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11a7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

// Below U+0300 every character has CCC=0 and NFC_Quick_Check=Yes.
pub(crate) const FIRST_NORMALIZATION_CODE_POINT: u32 = 0x0300;
// The first primary composite is U+00C0 LATIN CAPITAL LETTER A WITH GRAVE.
const FIRST_COMPOSITE: u32 = 0x00c0;
// Pack each code point with its Canonical_Combining_Class while normalizing.
const CLASS_SHIFT: u32 = 21;
const CODE_POINT_MASK: u32 = (1 << CLASS_SHIFT) - 1;

pub(crate) fn lookup(table: &[u32], shift: u32, cp: u32) -> u32 {
    // Find the last entry starting at or before cp; tables begin at U+0000.
    table[table.partition_point(|&entry| entry >> shift <= cp) - 1]
}

fn combining_class(cp: u32) -> u32 {
    if cp < FIRST_NORMALIZATION_CODE_POINT {
        0
    } else {
        lookup(NORMALIZATION, 10, cp) & 0xff
    }
}

pub(crate) fn is_nfc(text: &str) -> bool {
    let mut last_class = 0;
    let mut maybe = false;
    // UAX #15, Section 9: only compare with NFC if quick checking returns MAYBE.
    for ch in text.chars() {
        let cp = u32::from(ch);
        if cp < FIRST_NORMALIZATION_CODE_POINT {
            last_class = 0;
            continue;
        }
        let entry = lookup(NORMALIZATION, 10, cp);
        let class = entry & 0xff;
        let quick_check = (entry >> 8) & 3;
        if (class != 0 && last_class > class) || quick_check == 2 {
            return false;
        }
        maybe |= quick_check == 1;
        last_class = class;
    }
    if !maybe {
        return true;
    }

    // Quick checking has already verified canonical order. Only a starter can
    // expand into a starter followed by a few non-starters, so insertion of the
    // decompositions in canonical order remains linear in the input length.
    let mut buffer = Vec::with_capacity(text.len());
    for ch in text.chars() {
        decompose(u32::from(ch), &mut buffer);
    }
    let count = compose(&mut buffer);
    // Compare NFC with the original without rewriting the caller's text.
    buffer[..count]
        .iter()
        .map(|&value| value & CODE_POINT_MASK)
        .eq(text.chars().map(u32::from))
}

fn decompose(cp: u32, buffer: &mut Vec<u32>) {
    if (S_BASE..S_BASE + S_COUNT).contains(&cp) {
        let s = cp - S_BASE;
        let t = s % T_COUNT;
        // Conjoining jamo are all starters.
        buffer.push(L_BASE + s / N_COUNT);
        buffer.push(V_BASE + (s % N_COUNT) / T_COUNT);
        if t != 0 {
            buffer.push(T_BASE + t);
        }
        return;
    }
    if cp >= FIRST_COMPOSITE
        && let Ok(index) = COMPOSITIONS.binary_search_by_key(&cp, |entry| entry[0])
    {
        decompose(COMPOSITIONS[index][1], buffer);
        decompose(COMPOSITIONS[index][2], buffer);
        return;
    }
    let class = combining_class(cp);
    let value = (class << CLASS_SHIFT) | cp;
    buffer.push(value);
    if class == 0 {
        return;
    }
    // Canonical Ordering Algorithm (Unicode, Section 3.11): move a non-starter
    // before preceding non-starters with a higher canonical combining class.
    let mut index = buffer.len() - 1;
    while index > 0 && buffer[index - 1] >> CLASS_SHIFT > class {
        buffer[index] = buffer[index - 1];
        index -= 1;
    }
    buffer[index] = value;
}

// Canonical Composition Algorithm (Unicode, Section 3.11).
fn compose(buffer: &mut [u32]) -> usize {
    let mut last_class = 0;
    let mut starter: Option<usize> = None;
    let mut written = 0;
    for i in 0..buffer.len() {
        let value = buffer[i];
        let cp = value & CODE_POINT_MASK;
        let class = value >> CLASS_SHIFT;
        // Only non-starters follow the last starter. Composition is unblocked
        // when adjacent to it, or when the last written class is smaller.
        if let Some(s) = starter
            && (s + 1 == written || last_class < class)
            && let Some(composite) = try_compose(buffer[s], cp)
        {
            // Every primary composite is a starter, with combining class 0.
            buffer[s] = composite;
            continue;
        }
        if class == 0 {
            starter = Some(written);
        }
        last_class = class;
        buffer[written] = value;
        written += 1;
    }
    written
}

fn try_compose(first: u32, second: u32) -> Option<u32> {
    if (L_BASE..L_BASE + L_COUNT).contains(&first) && (V_BASE..V_BASE + V_COUNT).contains(&second) {
        return Some(S_BASE + ((first - L_BASE) * V_COUNT + second - V_BASE) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&first)
        && (first - S_BASE).is_multiple_of(T_COUNT)
        && (T_BASE + 1..T_BASE + T_COUNT).contains(&second)
    {
        return Some(first + second - T_BASE);
    }
    COMPOSITION_PAIRS
        .binary_search_by_key(&(first, second), |&index| {
            let entry = COMPOSITIONS[usize::from(index)];
            (entry[1], entry[2])
        })
        .ok()
        .map(|index| COMPOSITIONS[usize::from(COMPOSITION_PAIRS[index])][0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashSet, fs, path::Path};

    #[test]
    fn hangul_and_supplementary_compositions() {
        for (text, expected) in [
            ("\u{ac00}", true),
            ("\u{ac01}", true),
            ("\u{1100}\u{1161}", false),
            ("\u{ac00}\u{11a8}", false),
            ("\u{ac01}\u{11a8}", true),
            ("\u{113c2}\u{113c2}", false),
            ("\u{113c5}", true),
        ] {
            assert_eq!(is_nfc(text), expected, "{text:?}");
        }
    }

    #[test]
    fn upstream_unicode_conformance_and_unlisted_scalars() {
        // UCD 18.0.0 NormalizationTest columns: c2=NFC(c1)=NFC(c2)=NFC(c3),
        // c4=NFC(c4)=NFC(c5). Unlisted single scalars are normalization-invariant.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join("unicode_normalization.txt");
        let text = fs::read_to_string(path).unwrap();
        let mut listed = HashSet::new();
        let mut count = 0;
        for line in text.lines() {
            if line.is_empty() || line.starts_with(['#', '@']) {
                continue;
            }
            let columns: Vec<String> = line
                .split(';')
                .take(5)
                .map(|column| {
                    column
                        .split_whitespace()
                        .map(|hex| char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap())
                        .collect()
                })
                .collect();
            assert_eq!(columns.len(), 5);
            let expected = [
                columns[0] == columns[1],
                true,
                columns[2] == columns[1],
                true,
                columns[4] == columns[3],
            ];
            for (column, expected) in columns.iter().zip(expected) {
                assert_eq!(is_nfc(column), expected, "{line}");
            }
            if columns[0].chars().count() == 1 {
                listed.insert(columns[0].chars().next().unwrap());
            }
            count += 1;
        }
        assert!(count > 19_000, "{count}");
        for cp in 0..=0x10ffff {
            if let Some(ch) = char::from_u32(cp)
                && !listed.contains(&ch)
            {
                assert!(is_nfc(ch.encode_utf8(&mut [0; 4])), "U+{cp:04X}");
            }
        }
    }
}

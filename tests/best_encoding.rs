// Includes cases adapted from MimeKit's IO/Filters/FilterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentEncoding::{self, Base64, Binary, EightBit, QuotedPrintable, SevenBit},
    EncodingConstraint,
    filters::{BestEncodingFilter, MimeFilter},
};

const CONSTRAINTS: [EncodingConstraint; 3] = [
    EncodingConstraint::SevenBit,
    EncodingConstraint::EightBit,
    EncodingConstraint::None,
];

fn decisions(filter: &BestEncodingFilter, width: usize) -> [ContentEncoding; 3] {
    CONSTRAINTS.map(|constraint| {
        filter
            .best_encoding_with_line_length(constraint, width)
            .unwrap()
    })
}

fn analyze(input: &[u8], size: usize) -> BestEncodingFilter {
    let mut filter = BestEncodingFilter::new();
    let mut output = b"prefix".to_vec();
    for chunk in input.chunks(size) {
        filter.filter(chunk, &mut output);
        filter.filter(b"", &mut output);
        assert_eq!(&output[..6], b"prefix");
        assert_eq!(&output[6..], chunk);
        output.truncate(6);
    }
    let before = decisions(&filter, 78);
    filter.finish(&mut output);
    filter.finish(&mut output);
    assert_eq!(output, b"prefix");
    assert_eq!(decisions(&filter, 78), before);
    filter
}

// Whole-input model, using the source's floating-point threshold for small
// inputs, independently of the incremental scanner and integer ratio.
fn reference(input: &[u8], width: usize) -> [ContentEncoding; 3] {
    let nul = input.contains(&0);
    let high = input.iter().filter(|&&byte| byte > 127).count();
    let threshold = (input.len() as f64 * (17.0 / 100.0)) as usize;
    let mut needs_encoding = false;
    for line in input.split_inclusive(|&byte| byte == b'\n') {
        let content = if let Some(content) = line.strip_suffix(b"\n") {
            needs_encoding |= line.starts_with(b"From ");
            content.strip_suffix(b"\r").unwrap_or(content)
        } else {
            line
        };
        needs_encoding |= content.len() > width;
    }
    let seven = match (nul, high, needs_encoding) {
        (true, _, _) => Base64,
        (_, high, _) if high != 0 && high >= threshold => Base64,
        (_, high, force) if high != 0 || force => QuotedPrintable,
        _ => SevenBit,
    };
    let eight = match (nul, needs_encoding, high) {
        (true, _, _) => Base64,
        (_, true, _) => QuotedPrintable,
        (_, _, high) if high != 0 => EightBit,
        _ => SevenBit,
    };
    let unrestricted = match (needs_encoding, nul, high) {
        (true, nul, high) if nul || high > threshold => Base64,
        (true, _, _) => QuotedPrintable,
        (_, true, _) => Binary,
        (_, _, high) if high != 0 => EightBit,
        _ => SevenBit,
    };
    [seven, eight, unrestricted]
}

fn assert_splits(input: &[u8], width: usize, expected: [ContentEncoding; 3]) {
    assert_eq!(reference(input, width), expected);
    for split in 0..=input.len() {
        let mut filter = BestEncodingFilter::new();
        let mut output = b"prefix".to_vec();
        filter.filter(&input[..split], &mut output);
        assert_eq!(decisions(&filter, width), reference(&input[..split], width));
        let mut cloned = filter.clone();
        let mut cloned_output = output.clone();
        filter.filter(&input[split..], &mut output);
        filter.finish(&mut output);
        for chunk in input[split..].chunks(1) {
            cloned.filter(chunk, &mut cloned_output);
            cloned.filter(b"", &mut cloned_output);
        }
        cloned.finish(&mut cloned_output);
        assert_eq!(&output[..6], b"prefix");
        assert_eq!(&output[6..], input);
        assert_eq!(cloned_output, output);
        assert_eq!(decisions(&filter, width), expected, "split {split}");
        assert_eq!(decisions(&cloned, width), expected, "clone at {split}");
    }
}

#[test]
fn mimekit_ascii_from_and_french_vectors() {
    let ascii = b"This is some ascii text to make sure that\nthe filter returns 7bit encoding...\n";
    let from = b"This text is meant to test that the filter will armor lines beginning with\nFrom (like mbox).\n";
    let french = concat!(
        "Wikip\u{e9}dia est un projet d\u{2019}encyclop\u{e9}die collective en ligne, ",
        "universelle, multilingue et fonctionnant sur le principe du wiki. ",
        "Wikip\u{e9}dia a pour objectif d\u{2019}offrir un contenu librement ",
        "r\u{e9}utilisable, objectif et v\u{e9}rifiable, que chacun peut modifier et ",
        "am\u{e9}liorer.\n\nTous les r\u{e9}dacteurs des articles de Wikip\u{e9}dia ",
        "sont b\u{e9}n\u{e9}voles. Ils coordonnent leurs efforts au sein d'une ",
        "communaut\u{e9} collaborative, sans dirigeant.",
    )
    .as_bytes();
    assert_splits(ascii, 78, [SevenBit; 3]);
    assert_splits(from, 78, [QuotedPrintable; 3]);
    assert_splits(&french[..60], 78, [QuotedPrintable, EightBit, EightBit]);
    assert_splits(french, 78, [QuotedPrintable; 3]);
}

#[test]
fn nul_high_bit_and_line_rules_have_source_precedence() {
    let cases: &[(&[u8], [ContentEncoding; 3])] = &[
        (b"", [SevenBit; 3]),
        (b"plain ASCII \t\r\n", [SevenBit; 3]),
        (b"\x7f", [SevenBit; 3]),
        (b"\0", [Base64, Base64, Binary]),
        (b"\xff\0", [Base64, Base64, Binary]),
        (b"\x80", [Base64, EightBit, EightBit]),
        (
            b"mostly ascii with \xff",
            [QuotedPrintable, EightBit, EightBit],
        ),
        (b"From \0\n", [Base64; 3]),
        (b"From \xff\n", [Base64, QuotedPrintable, QuotedPrintable]),
        (b"From \xff\xff\n", [Base64, QuotedPrintable, Base64]),
    ];
    for &(input, expected) in cases {
        assert_splits(input, 78, expected);
    }
    let mut long = vec![b'x'; 100];
    long[0] = 0;
    assert_splits(&long, 78, [Base64; 3]);
    long[0] = 0xff;
    assert_splits(&long, 78, [QuotedPrintable; 3]);
    long.fill(0xff);
    assert_splits(&long, 78, [Base64, QuotedPrintable, Base64]);
}

#[test]
fn seventeen_percent_boundary_differs_between_constraints() {
    for high in [16, 17, 18] {
        let mut input = vec![b'a'; 100];
        input[5..5 + high].fill(0xff);
        let seven = if high >= 17 { Base64 } else { QuotedPrintable };
        let unrestricted = if high > 17 { Base64 } else { QuotedPrintable };
        assert_splits(&input, 998, [seven, EightBit, EightBit]);
        assert_splits(&input, 78, [seven, QuotedPrintable, unrestricted]);

        // Include both newline bytes in the denominator, not just text bytes.
        input[..5].copy_from_slice(b"From ");
        input[98..].copy_from_slice(b"\r\n");
        assert_splits(&input, 998, [seven, QuotedPrintable, unrestricted]);
    }

    // The source compares against floor(total * 0.17), not an exact density.
    assert_splits(b"\xffaaaaa", 78, [Base64, EightBit, EightBit]);
    assert_splits(
        b"\xffaaaaaaaaaaa",
        78,
        [QuotedPrintable, EightBit, EightBit],
    );
}

#[test]
fn every_small_ratio_matches_the_source_threshold() {
    for total in 1..=200 {
        for high in 0..=total {
            let mut input = vec![b'a'; total];
            input[..high].fill(0x80);
            let filter = analyze(&input, 7);
            for width in [60, 78, 998] {
                assert_eq!(
                    decisions(&filter, width),
                    reference(&input, width),
                    "total {total}, high {high}, width {width}",
                );
            }
        }
    }
}

#[test]
fn crlf_splits_and_unterminated_lines_preserve_source_widths() {
    let mut filter = BestEncodingFilter::new();
    let mut output = Vec::new();
    let mut input = vec![b'x'; 78];
    input.push(b'\r');
    filter.filter(&input, &mut output);
    assert_eq!(decisions(&filter, 78), [QuotedPrintable; 3]);
    filter.finish(&mut output);
    assert_eq!(decisions(&filter, 78), [QuotedPrintable; 3]);
    filter.filter(b"\n", &mut output);
    filter.finish(&mut output);
    assert_eq!(decisions(&filter, 78), [SevenBit; 3]);
    assert_eq!(output, [input, vec![b'\n']].concat());

    filter.reset();
    filter.filter(&[b'x'; 79], &mut Vec::new());
    assert_eq!(decisions(&filter, 78), [QuotedPrintable; 3]);
    assert_splits(
        b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz\r\nabc\r\n",
        78,
        [SevenBit; 3],
    );

    for width in [60, 76, 78, 998] {
        for length in [width - 1, width, width + 1] {
            for suffix in [b"".as_slice(), b"\n", b"\r\n", b"\r", b"\r\r\n"] {
                let input = [vec![b'x'; length], suffix.to_vec()].concat();
                let extra_cr = usize::from(matches!(suffix, b"\r" | b"\r\r\n"));
                let encoding = if length + extra_cr > width {
                    QuotedPrintable
                } else {
                    SevenBit
                };
                for size in [1, 7, length, input.len() + 1] {
                    assert_eq!(decisions(&analyze(&input, size), width), [encoding; 3]);
                }
            }
        }
    }
}

#[test]
fn from_markers_are_exact_lf_terminated_and_latched() {
    for input in [
        b"From ".as_slice(),
        b"From x\r",
        b"from x\n",
        b"FROM x\n",
        b">From x\n",
        b" From x\n",
        b"x\rFrom x\n",
        b"From\t\n",
        b"F\nrom \n",
        b"\n\n\r\n",
    ] {
        assert_splits(input, 78, [SevenBit; 3]);
    }
    for input in [b"From \n".as_slice(), b"From x\r\n", b"\nFrom x\nshort\n"] {
        assert_splits(input, 78, [QuotedPrintable; 3]);
    }

    let mut filter = analyze(b"From x\r", 1);
    assert_eq!(decisions(&filter, 78), [SevenBit; 3]);
    let mut clone = filter.clone();
    filter.filter(b"\n", &mut Vec::new());
    assert_eq!(decisions(&filter, 78), [QuotedPrintable; 3]);
    clone.filter(b"x", &mut Vec::new());
    assert_eq!(decisions(&clone, 78), [SevenBit; 3]);
    filter.filter(b"short\n", &mut Vec::new());
    assert_eq!(decisions(&filter, 78), [QuotedPrintable; 3]);
}

#[test]
fn every_byte_is_passed_through_and_classified() {
    for byte in 0..=255 {
        let expected = match byte {
            0 => [Base64, Base64, Binary],
            128..=255 => [Base64, EightBit, EightBit],
            _ => [SevenBit; 3],
        };
        assert_splits(&[byte], 78, expected);
        for position in 0..5 {
            let mut input = b"From \n".to_vec();
            input[position] = byte;
            assert_splits(&input, 78, reference(&input, 78));
        }
    }
}

#[test]
fn short_input_partitions_match_whole_input_analysis() {
    let alphabet = b"x\r\n\0\xff";
    for length in 0..=5u32 {
        for mut value in 0..alphabet.len().pow(length) {
            let mut input = vec![0; length as usize];
            for byte in &mut input {
                *byte = alphabet[value % alphabet.len()];
                value /= alphabet.len();
            }
            for cuts in 0..1 << input.len().saturating_sub(1) {
                let mut filter = BestEncodingFilter::new();
                let mut output = Vec::new();
                let mut start = 0;
                for end in 1..=input.len() {
                    if end == input.len() || cuts & (1 << (end - 1)) != 0 {
                        filter.filter(&input[start..end], &mut output);
                        filter.filter(b"", &mut output);
                        start = end;
                    }
                }
                assert_eq!(decisions(&filter, 78), reference(&input, 78));
                assert_eq!(output, input);
            }
        }
    }
}

#[test]
fn reset_clears_counts_line_state_and_partial_markers() {
    let mut filter = analyze(b"From \0\xff\n", 1);
    filter.reset();
    assert_eq!(decisions(&filter, 78), [SevenBit; 3]);
    filter.filter(b"Fro", &mut Vec::new());
    filter.reset();
    filter.filter(b"m \n", &mut Vec::new());
    assert_eq!(decisions(&filter, 78), [SevenBit; 3]);
    filter.filter(&[b'x'; 1000], &mut Vec::new());
    filter.filter(b"\r", &mut Vec::new());
    assert_eq!(decisions(&filter, 998), [QuotedPrintable; 3]);
    filter.reset();
    filter.filter(b"\n", &mut Vec::new());
    assert_eq!(decisions(&filter, 60), [SevenBit; 3]);
}

#[test]
fn line_limits_are_validated_without_capping_or_changing_state() {
    assert_eq!(EncodingConstraint::default(), EncodingConstraint::None);
    let filter = analyze(&[b'x'; 79], 3);
    for constraint in CONSTRAINTS {
        assert_eq!(filter.best_encoding(constraint), QuotedPrintable);
        for width in [0, 59, 999, usize::MAX] {
            let error = filter
                .best_encoding_with_line_length(constraint, width)
                .unwrap_err();
            assert_eq!(error.requested(), width);
        }
        assert_eq!(
            filter.best_encoding_with_line_length(constraint, 60),
            Ok(QuotedPrintable)
        );
        assert_eq!(
            filter.best_encoding_with_line_length(constraint, 79),
            Ok(SevenBit)
        );
        assert_eq!(
            filter.best_encoding_with_line_length(constraint, 998),
            Ok(SevenBit)
        );
        assert_eq!(filter.best_encoding(constraint), QuotedPrintable);
    }
}

#[test]
fn generic_and_boxed_filters_preserve_analysis_and_finish_state() {
    fn feed<F: MimeFilter>(filter: &mut F, input: &[u8], output: &mut Vec<u8>) {
        filter.filter(input, output);
        filter.finish(output);
    }
    let mut filter = BestEncodingFilter::new();
    let mut output = b"prefix".to_vec();
    let mut boxed: Box<dyn MimeFilter> = Box::new(filter.clone());
    feed(&mut boxed, b"From \0\n", &mut output);
    boxed.reset();
    feed(&mut boxed, b"other", &mut output);
    assert_eq!(output, b"prefixFrom \0\nother");
    feed(&mut filter, b"Fr", &mut output);
    assert_eq!(decisions(&filter, 78), [SevenBit; 3]);
    let erased: &mut dyn MimeFilter = &mut filter;
    erased.filter(b"om \n", &mut output);
    erased.finish(&mut output);
    assert_eq!(decisions(&filter, 78), [QuotedPrintable; 3]);
}

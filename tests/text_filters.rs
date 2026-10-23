// Includes cases adapted from MimeKit's IO/Filters/FilterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    encodings::{QuotedPrintableDecoder, QuotedPrintableEncoder},
    filters::{
        ArmoredFromFilter, EncoderFilter, MboxFromFilter, MimeFilter, TrailingWhitespaceFilter,
    },
};

fn run<F: MimeFilter + ?Sized>(filter: &mut F, input: &[u8], size: usize) -> Vec<u8> {
    let mut buffer = b"prefix".to_vec();
    let mut output = Vec::new();
    for chunk in input.chunks(size) {
        filter.filter(chunk, &mut buffer);
        filter.filter(b"", &mut buffer);
        assert_eq!(&buffer[..6], b"prefix");
        output.extend_from_slice(&buffer[6..]);
        buffer.truncate(6);
    }
    filter.finish(&mut buffer);
    let finished_length = buffer.len();
    filter.finish(&mut buffer);
    assert_eq!(buffer.len(), finished_length);
    assert_eq!(&buffer[..6], b"prefix");
    output.extend_from_slice(&buffer[6..]);
    output
}

fn assert_splits<F: MimeFilter + Clone>(template: F, input: &[u8], expected: &[u8]) {
    for split in 0..=input.len() {
        let mut filter = template.clone();
        let mut output = b"prefix".to_vec();
        filter.filter(&input[..split], &mut output);
        filter.filter(b"", &mut output);
        let mut clone = filter.clone();
        let mut cloned_output = output.clone();
        let mut reset = filter.clone();
        reset.reset();
        assert_eq!(run(&mut reset, input, 3), expected);

        filter.filter(&input[split..], &mut output);
        filter.finish(&mut output);
        let erased: &mut dyn MimeFilter = &mut clone;
        for chunk in input[split..].chunks(1) {
            erased.filter(chunk, &mut cloned_output);
        }
        erased.finish(&mut cloned_output);
        assert_eq!(&output[..6], b"prefix");
        assert_eq!(&output[6..], expected, "input {input:?}, split {split}");
        assert_eq!(cloned_output, output);
    }
}

// Whole-line models are independent of the incremental marker scanner and
// run-length buffering used by the implementation.
fn reference_from(input: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for line in input.split_inclusive(|&byte| byte == b'\n') {
        if let Some(rest) = line.strip_prefix(b"From ") {
            output.extend_from_slice(replacement);
            output.extend_from_slice(rest);
        } else {
            output.extend_from_slice(line);
        }
    }
    output
}

fn reference_trim(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for line in input.split_inclusive(|&byte| matches!(byte, b'\r' | b'\n')) {
        let has_newline = matches!(line.last(), Some(b'\r' | b'\n'));
        let end = line.len() - usize::from(has_newline);
        let mut trimmed_end = end;
        while trimmed_end > 0 && matches!(line[trimmed_end - 1], b' ' | b'\t') {
            trimmed_end -= 1;
        }
        output.extend_from_slice(&line[..trimmed_end]);
        if has_newline {
            output.push(line[end]);
        }
    }
    output
}

#[test]
fn mimekit_from_vectors_at_every_split() {
    let input = b"This text is meant to test that the filter will armor lines beginning with\n\
From (like mbox). And let's add another\nFrom line for good measure, shall we?\n";
    let armored = b"This text is meant to test that the filter will armor lines beginning with\n\
=46rom (like mbox). And let's add another\n=46rom line for good measure, shall we?\n";
    let escaped = b"This text is meant to test that the filter will armor lines beginning with\n\
>From (like mbox). And let's add another\n>From line for good measure, shall we?\n";
    assert_eq!(reference_from(input, b"=46rom "), armored);
    assert_eq!(reference_from(input, b">From "), escaped);
    assert_splits(ArmoredFromFilter::new(), input, armored);
    assert_splits(MboxFromFilter::new(), input, escaped);
}

#[test]
fn repeated_mimekit_from_russia_fixture() {
    let line = b"From Russia with love is one of my favorite James Bond files.\n";
    let input = line.repeat(1000);
    let escaped = b">From Russia with love is one of my favorite James Bond files.\n".repeat(1000);
    let armored = b"=46rom Russia with love is one of my favorite James Bond files.\n".repeat(1000);
    for size in [1, 8, 64, 1024] {
        let mut mbox = MboxFromFilter::new();
        let mut armor = ArmoredFromFilter::new();
        assert_eq!(run(&mut mbox, &input, size), escaped);
        assert_eq!(run(&mut armor, &input, size), armored);
        mbox.reset();
        armor.reset();
        assert_eq!(run(&mut mbox, &input, size), escaped);
        assert_eq!(run(&mut armor, &input, size), armored);
    }
}

#[test]
fn from_matching_is_exact_and_only_lf_starts_another_line() {
    let cases: &[&[u8]] = &[
        b"",
        b"F",
        b"Fr",
        b"Fro",
        b"From",
        b"From ",
        b"From \nFrom \r\nFrom ",
        b"from lower\nFROM upper\n From indented\n>From quoted\n>>From quoted twice",
        b"xFrom middle\nFrom\tno-space\nFrom\r\nFrom\nFrom!",
        b"\rFrom after-bare-cr\r\nFrom after-crlf\nFrom after-lf",
        b"F\nFr\nFro\nFrom\nFrom next\nFrom",
        b"\0From hidden\nFrom \xff\0\r\n\xffFrom non-ascii\nFrom ",
    ];
    for &input in cases {
        assert_splits(
            MboxFromFilter::new(),
            input,
            &reference_from(input, b">From "),
        );
        assert_splits(
            ArmoredFromFilter::new(),
            input,
            &reference_from(input, b"=46rom "),
        );
    }
    assert_eq!(
        run(&mut MboxFromFilter::new(), b"From original", 1),
        run(&mut MboxFromFilter::new(), b">From original", 1),
    );
}

#[test]
fn every_byte_at_each_marker_position_matches_whole_line_reference() {
    for position in 0..5 {
        for byte in 0..=255 {
            let mut input = b"From \nFrom final".to_vec();
            input[position] = byte;
            assert_splits(
                MboxFromFilter::new(),
                &input,
                &reference_from(&input, b">From "),
            );
            assert_splits(
                ArmoredFromFilter::new(),
                &input,
                &reference_from(&input, b"=46rom "),
            );
        }
    }
}

fn assert_from_lifecycle<F: MimeFilter + Clone>(template: F, replacement: &[u8]) {
    for length in 0..=4 {
        let mut filter = template.clone();
        let mut output = Vec::new();
        filter.filter(&b"From "[..length], &mut output);
        filter.filter(b"", &mut output);
        assert!(output.is_empty());
        let mut clone = filter.clone();

        filter.filter(&b"From "[length..], &mut output);
        assert_eq!(output, replacement);
        output.clear();
        filter.finish(&mut output);
        assert!(output.is_empty());

        clone.finish(&mut output);
        assert_eq!(output, &b"From "[..length]);
        clone.finish(&mut output);
        assert_eq!(output, &b"From "[..length]);
        if length != 0 {
            clone.filter(b"From ", &mut output);
            let expected = [&b"From "[..length], b"From "].concat();
            assert_eq!(output, expected);
        }

        clone.reset();
        output.clear();
        clone.filter(b"Fr", &mut output);
        clone.reset();
        clone.filter(b"From ", &mut output);
        assert_eq!(output, replacement);
    }
}

#[test]
fn pending_from_prefixes_finish_literally_and_reset_returns_to_line_start() {
    assert_from_lifecycle(MboxFromFilter::new(), b">From ");
    assert_from_lifecycle(ArmoredFromFilter::new(), b"=46rom ");
}

#[test]
fn trailing_whitespace_vectors_and_all_split_clone_states() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"", b""),
        (b" \t ", b""),
        (b"  indent \ttext\t ", b"  indent \ttext"),
        (b"one \t\r\ntwo\t \nthree \r", b"one\r\ntwo\nthree\r"),
        (b" \r \n \r\n", b"\r\n\r\n"),
        (b"word \rnext", b"word\rnext"),
        (b"x \t\x0b \t\x0c \t", b"x \t\x0b \t\x0c"),
        (b"\t\x00 \xff \x7f\t ", b"\t\x00 \xff \x7f"),
        (b" \tX\t Y\t Z ", b" \tX\t Y\t Z"),
    ];
    for &(input, expected) in cases {
        assert_eq!(reference_trim(input), expected);
        assert_splits(TrailingWhitespaceFilter::new(), input, expected);
    }
}

#[test]
fn every_two_byte_input_has_only_spaces_and_tabs_removed() {
    for first in 0..=255 {
        for second in 0..=255 {
            let input = [first, second];
            assert_eq!(
                run(&mut TrailingWhitespaceFilter::new(), &input, 1),
                reference_trim(&input),
                "input {input:?}",
            );
        }
    }
}

#[test]
fn whitespace_transitions_at_every_short_input_partition() {
    let alphabet = b" \t\r\nx\0\xff";
    for length in 0..=4u32 {
        for mut value in 0..alphabet.len().pow(length) {
            let mut input = vec![0; length as usize];
            for byte in &mut input {
                *byte = alphabet[value % alphabet.len()];
                value /= alphabet.len();
            }
            let expected = reference_trim(&input);
            for cuts in 0..1 << input.len().saturating_sub(1) {
                let mut filter = TrailingWhitespaceFilter::new();
                let mut output = b"prefix".to_vec();
                let mut start = 0;
                for end in 1..=input.len() {
                    if end == input.len() || cuts & (1 << (end - 1)) != 0 {
                        filter.filter(&input[start..end], &mut output);
                        filter.filter(b"", &mut output);
                        start = end;
                    }
                }
                filter.finish(&mut output);
                filter.finish(&mut output);
                assert_eq!(&output[..6], b"prefix");
                assert_eq!(&output[6..], expected, "input {input:?}, cuts {cuts}");
            }
        }
    }
}

#[test]
fn long_and_alternating_whitespace_runs_survive_chunk_boundaries() {
    for count in [254, 255, 256, 510, 511, 4096, 65536] {
        let mut input = b"start".to_vec();
        input.extend(std::iter::repeat_n(b' ', count));
        input.extend(std::iter::repeat_n(b'\t', count));
        input.extend(b" \t".repeat(count));
        input.extend_from_slice(b"end \t\r\ntrailing ");
        input.extend(std::iter::repeat_n(b'\t', count));
        let expected = reference_trim(&input);
        for size in [1, 254, 255, 256, 4096, input.len() + 1] {
            assert_eq!(
                run(&mut TrailingWhitespaceFilter::new(), &input, size),
                expected,
            );
        }
    }
}

#[test]
fn whitespace_finish_reset_and_clone_have_independent_pending_runs() {
    let mut filter = TrailingWhitespaceFilter::new();
    let mut output = b"prefix".to_vec();
    let blanks = [vec![b' '; 256], vec![b'\t'; 256]].concat();
    filter.filter(&blanks, &mut output);
    filter.filter(b"", &mut output);
    assert_eq!(output, b"prefix");
    let mut cloned = filter.clone();

    filter.filter(b"\r", &mut output);
    assert_eq!(output, b"prefix\r");
    let expected = [blanks, b"x".to_vec()].concat();
    assert_eq!(run(&mut cloned, b"x", 1), expected);

    filter.filter(b" \t", &mut output);
    filter.finish(&mut output);
    filter.filter(b"x", &mut output);
    assert_eq!(output, b"prefix\rx");
    filter.filter(b"\t ", &mut output);
    filter.reset();
    filter.filter(b"y", &mut output);
    assert_eq!(output, b"prefix\rxy");
}

#[test]
fn boxed_text_filters_use_the_common_interface() {
    let cases: Vec<(Box<dyn MimeFilter>, &[u8])> = vec![
        (Box::new(MboxFromFilter::new()), b">From one \t\nFrom"),
        (Box::new(ArmoredFromFilter::new()), b"=46rom one \t\nFrom"),
        (Box::new(TrailingWhitespaceFilter::new()), b"From one\nFrom"),
    ];
    for (mut filter, expected) in cases {
        assert_eq!(run(&mut filter, b"From one \t\nFrom", 1), expected);
        filter.reset();
        assert_eq!(run(&mut filter, b"From one \t\nFrom", 3), expected);
    }
}

#[test]
fn armoring_after_quoted_printable_encoding_remains_decodable() {
    let mut input = b"From ".to_vec();
    input.extend(std::iter::repeat_n(b'a', 300));
    input.extend_from_slice(b"\r\nFrom \xff\t =second\r\nmid From middle\r\nFrom");
    for size in [1, 2, 3, 7, 64, input.len() + 1] {
        // Reserve two columns for the =46 replacement of the initial F.
        let encoder = QuotedPrintableEncoder::with_line_length(74).unwrap();
        let mut encoder = EncoderFilter::new(encoder);
        let mut armor = ArmoredFromFilter::new();
        let mut intermediate = Vec::new();
        let mut output = Vec::new();
        for chunk in input.chunks(size) {
            encoder.filter(chunk, &mut intermediate);
            armor.filter(&intermediate, &mut output);
            intermediate.clear();
        }
        encoder.finish(&mut intermediate);
        armor.filter(&intermediate, &mut output);
        armor.finish(&mut output);
        assert!(output.starts_with(b"=46rom "));
        for line in output.split_inclusive(|&byte| byte == b'\n') {
            let line = line.strip_suffix(b"\r\n").unwrap();
            assert!(line.len() <= 76);
            assert!(!line.starts_with(b"From "));
        }

        let mut decoder = QuotedPrintableDecoder::new();
        let mut decoded = Vec::new();
        for chunk in output.chunks(3) {
            decoder.decode(chunk, &mut decoded);
        }
        assert_eq!(decoded, input);
    }
}

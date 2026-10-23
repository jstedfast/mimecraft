// Includes cases adapted from MimeKit's IO/Filters unit tests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentEncoding,
    encodings::{
        Base64Decoder, Base64Encoder, EncoderOptionsError, MimeDecoder, MimeEncoder, NewLineFormat,
        Rfc2231Decoder, UuEncoder, YEncoder,
    },
    filters::{
        DecoderFilter, Dos2UnixFilter, EncoderFilter, MimeFilter, PassThroughFilter,
        Unix2DosFilter, decoder_filter, encoder_filter, encoder_filter_with_options,
    },
};

fn run<F: MimeFilter + ?Sized>(filter: &mut F, input: &[u8], size: usize) -> Vec<u8> {
    let mut output = b"prefix".to_vec();
    for chunk in input.chunks(size) {
        filter.filter(chunk, &mut output);
        filter.filter(b"", &mut output);
    }
    filter.finish(&mut output);
    assert_eq!(&output[..6], b"prefix");
    output[6..].to_vec()
}

// Independent whole-input models: add any requested final LF first, then use
// lookahead/lookbehind instead of a streaming previous-byte state machine.
fn reference_dos_to_unix(input: &[u8], ensure: bool) -> Vec<u8> {
    let mut input = input.to_vec();
    if ensure && !input.ends_with(b"\n") {
        input.push(b'\n');
    }
    input
        .iter()
        .enumerate()
        .filter_map(|(index, &byte)| {
            (byte != b'\r' || input.get(index + 1) != Some(&b'\n')).then_some(byte)
        })
        .collect()
}

fn reference_unix_to_dos(input: &[u8], ensure: bool) -> Vec<u8> {
    let mut input = input.to_vec();
    if ensure && !input.ends_with(b"\n") {
        input.push(b'\n');
    }
    let mut output = Vec::new();
    for (index, &byte) in input.iter().enumerate() {
        if byte == b'\n' && (index == 0 || input[index - 1] != b'\r') {
            output.push(b'\r');
        }
        output.push(byte);
    }
    output
}

fn assert_partitions<F: MimeFilter + Clone>(template: F, input: &[u8], expected: &[u8]) {
    for cuts in 0..1 << input.len().saturating_sub(1) {
        let mut filter = template.clone();
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

#[test]
fn mimekit_newline_vectors_and_repeated_finish() {
    let dos_cases: &[(&[u8], &[u8], &[u8])] = &[
        (b"", b"", b"\n"),
        (b"abc", b"abc", b"abc\n"),
        (b"abc\r", b"abc\r", b"abc\n"),
        (b"abc\r\r", b"abc\r\r", b"abc\r\n"),
        (b"abc\r\n\r", b"abc\n\r", b"abc\n\n"),
        (b"\r", b"\r", b"\n"),
        (
            b"one\r\ntwo\r\nthree",
            b"one\ntwo\nthree",
            b"one\ntwo\nthree\n",
        ),
        (
            b"one\ntwo\r\nthree\n",
            b"one\ntwo\nthree\n",
            b"one\ntwo\nthree\n",
        ),
    ];
    let unix_cases: &[(&[u8], &[u8], &[u8])] = &[
        (b"", b"", b"\r\n"),
        (b"abc", b"abc", b"abc\r\n"),
        (b"abc\r", b"abc\r", b"abc\r\n"),
        (b"abc\r\r", b"abc\r\r", b"abc\r\r\n"),
        (b"\r", b"\r", b"\r\n"),
        (
            b"one\ntwo\nthree",
            b"one\r\ntwo\r\nthree",
            b"one\r\ntwo\r\nthree\r\n",
        ),
        (
            b"one\ntwo\r\nthree\r",
            b"one\r\ntwo\r\nthree\r",
            b"one\r\ntwo\r\nthree\r\n",
        ),
    ];
    for ensure in [false, true] {
        for &(input, unchanged_end, with_newline) in dos_cases {
            let expected = if ensure { with_newline } else { unchanged_end };
            assert_eq!(reference_dos_to_unix(input, ensure), expected);
            let mut filter = Dos2UnixFilter::with_ensure_new_line(ensure);
            for size in 1..=input.len() + 1 {
                assert_eq!(run(&mut filter, input, size), expected);
                let mut tail = Vec::new();
                filter.finish(&mut tail);
                assert!(tail.is_empty());
                filter.reset();
                assert_eq!(filter.ensure_new_line(), ensure);
            }
        }
        for &(input, unchanged_end, with_newline) in unix_cases {
            let expected = if ensure { with_newline } else { unchanged_end };
            assert_eq!(reference_unix_to_dos(input, ensure), expected);
            let mut filter = Unix2DosFilter::with_ensure_new_line(ensure);
            for size in 1..=input.len() + 1 {
                assert_eq!(run(&mut filter, input, size), expected);
                let mut tail = Vec::new();
                filter.finish(&mut tail);
                assert!(tail.is_empty());
                filter.reset();
                assert_eq!(filter.ensure_new_line(), ensure);
            }
        }
    }
}

#[test]
fn exhaustive_short_newline_inputs_at_every_chunk_partition() {
    let alphabet = b"x\r\n\0\xff";
    for length in 0..=5u32 {
        for mut value in 0..alphabet.len().pow(length) {
            let mut input = vec![0; length as usize];
            for byte in &mut input {
                *byte = alphabet[value % alphabet.len()];
                value /= alphabet.len();
            }
            for ensure in [false, true] {
                assert_partitions(
                    Dos2UnixFilter::with_ensure_new_line(ensure),
                    &input,
                    &reference_dos_to_unix(&input, ensure),
                );
                assert_partitions(
                    Unix2DosFilter::with_ensure_new_line(ensure),
                    &input,
                    &reference_unix_to_dos(&input, ensure),
                );
            }
        }
    }
}

#[test]
fn newline_clones_and_resets_preserve_configuration_not_pending_input() {
    assert!(!Dos2UnixFilter::default().ensure_new_line());
    assert!(!Unix2DosFilter::default().ensure_new_line());
    for ensure in [false, true] {
        let mut dos = Dos2UnixFilter::with_ensure_new_line(ensure);
        let mut output = Vec::new();
        dos.filter(b"\r", &mut output);
        assert!(output.is_empty());
        let mut clone = dos.clone();
        assert_eq!(run(&mut dos, b"\n", 1), b"\n");
        assert_eq!(
            run(&mut clone, b"x", 1),
            if ensure { &b"\rx\n"[..] } else { b"\rx" }
        );

        dos.filter(b"\r", &mut output);
        dos.reset();
        assert_eq!(
            run(&mut dos, b"x", 1),
            if ensure { &b"x\n"[..] } else { b"x" }
        );
        assert_eq!(dos.ensure_new_line(), ensure);

        let mut unix = Unix2DosFilter::with_ensure_new_line(ensure);
        output.clear();
        unix.filter(b"\r", &mut output);
        assert_eq!(output, b"\r");
        let mut clone = unix.clone();
        assert_eq!(run(&mut unix, b"\n", 1), b"\n");
        assert_eq!(
            run(&mut clone, b"x", 1),
            if ensure { &b"x\r\n"[..] } else { b"x" }
        );

        unix.filter(b"\r", &mut output);
        unix.reset();
        assert_eq!(run(&mut unix, b"\n", 1), b"\r\n");
        assert_eq!(unix.ensure_new_line(), ensure);
    }
}

#[test]
fn every_byte_is_preserved_except_for_the_requested_newline_conversion() {
    let input: Vec<u8> = (0..=255).cycle().take(1024).collect();
    for size in [1, 2, 3, 7, 255, 1024, 1025] {
        let mut pass = PassThroughFilter::new();
        assert_eq!(run(&mut pass, &input[1..1023], size), input[1..1023]);
        pass.reset();
        assert_eq!(run(&mut pass, b"", size), b"");
        for ensure in [false, true] {
            assert_eq!(
                run(
                    &mut Dos2UnixFilter::with_ensure_new_line(ensure),
                    &input,
                    size
                ),
                reference_dos_to_unix(&input, ensure),
            );
            assert_eq!(
                run(
                    &mut Unix2DosFilter::with_ensure_new_line(ensure),
                    &input,
                    size
                ),
                reference_unix_to_dos(&input, ensure),
            );
        }
    }
}

#[test]
fn encoder_adapter_tracks_only_its_own_output_and_preserves_state() {
    let encoder = Base64Encoder::with_options(60, NewLineFormat::Unix).unwrap();
    let mut filter = EncoderFilter::new(encoder);
    let mut output = b"prefix".to_vec();
    assert_eq!(filter.encoding(), ContentEncoding::Base64);
    assert!(!filter.has_output());
    filter.filter(b"", &mut output);
    filter.filter(b"C", &mut output);
    assert!(!filter.has_output());
    assert_eq!(output, b"prefix");

    let mut clone = filter.clone();
    filter.filter(b"at", &mut output);
    assert!(filter.has_output());
    output.clear();
    filter.filter(b"", &mut output);
    assert!(filter.has_output());
    filter.finish(&mut output);
    assert_eq!(output, b"\n");

    assert_eq!(run(&mut clone, b"at", 1), b"Q2F0\n");
    assert!(clone.has_output());
    let cloned_after_output = clone.clone();
    assert!(cloned_after_output.has_output());

    MimeFilter::reset(&mut filter);
    assert!(!filter.has_output());
    assert_eq!(filter.encoder().line_length(), 60);
    assert_eq!(filter.encoder().new_line_format(), NewLineFormat::Unix);
    filter.filter(b"C", &mut output);
    assert!(!filter.has_output());
    output.clear();
    filter.finish(&mut output);
    assert_eq!(output, b"Qw==\n");
    assert!(filter.has_output());

    filter.reset();
    filter.encoder_mut().encode(b"C", &mut output);
    assert!(!filter.has_output());
    let encoder = filter.into_inner();
    let mut wrapped = EncoderFilter::new(encoder);
    assert_eq!(run(&mut wrapped, b"at", 1), b"Q2F0\n");
}

#[test]
fn decoder_finish_retains_incomplete_input_and_into_inner_retains_state() {
    let mut filter = DecoderFilter::new(Rfc2231Decoder::new());
    assert_eq!(filter.encoding(), ContentEncoding::Default);
    let mut output = Vec::new();
    filter.filter(b"%4", &mut output);
    filter.finish(&mut output);
    filter.finish(&mut output);
    assert!(output.is_empty());
    let mut cloned = filter.clone();
    filter.filter(b"3", &mut output);
    assert_eq!(output, b"C");
    assert_eq!(run(&mut cloned, b"1", 1), b"A");

    filter.reset();
    filter.decoder_mut().decode(b"%6", &mut output);
    let decoder = filter.into_inner();
    let mut wrapped = DecoderFilter::new(decoder);
    assert_eq!(run(&mut wrapped, b"1", 1), b"a");
    wrapped.filter(b"%", &mut output);
    MimeFilter::reset(&mut wrapped);
    assert_eq!(run(&mut wrapped, b"%43", 1), b"C");
}

#[test]
fn decoder_finish_delegates_empty_input_to_custom_decoders() {
    #[derive(Default)]
    struct EmptyInputDecoder(usize);
    impl MimeDecoder for EmptyInputDecoder {
        fn encoding(&self) -> ContentEncoding {
            ContentEncoding::Default
        }
        fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
            if input.is_empty() {
                self.0 += 1;
                output.push(b'!');
            } else {
                output.extend_from_slice(input);
            }
        }
        fn reset(&mut self) {
            self.0 = 0;
        }
    }
    let mut filter = DecoderFilter::new(EmptyInputDecoder::default());
    let mut output = Vec::new();
    filter.filter(b"body", &mut output);
    MimeFilter::finish(&mut filter, &mut output);
    assert_eq!(output, b"body!");
    assert_eq!(filter.decoder().0, 1);
    filter.reset();
    assert_eq!(filter.decoder().0, 0);
}

#[test]
fn adapter_finish_keeps_uu_terminators_and_yenc_checksum_lifetime() {
    let mut uu = EncoderFilter::new(UuEncoder::new());
    let mut output = Vec::new();
    uu.finish(&mut output);
    uu.finish(&mut output);
    assert_eq!(output, b"`\r\n`\r\n");
    assert!(uu.has_output());
    uu.reset();
    assert!(!uu.has_output());

    let mut yenc = EncoderFilter::new(YEncoder::new());
    let output = run(&mut yenc, b"123456789", 1);
    let checksum = yenc.encoder().checksum();
    assert_eq!(!checksum, 0xcbf43926);
    let mut tail = Vec::new();
    yenc.finish(&mut tail);
    assert!(tail.is_empty());
    assert!(!output.is_empty());
    assert_eq!(yenc.encoder().checksum(), checksum);
    assert_eq!(yenc.encoding(), ContentEncoding::Default);
    yenc.reset();
    assert_eq!(yenc.encoder().checksum(), u32::MAX);
    assert!(!yenc.has_output());
}

#[test]
fn boxed_codecs_and_boxed_filters_support_generic_dispatch() {
    let encoder: Box<dyn MimeEncoder> = Box::new(Base64Encoder::new());
    let decoder: Box<dyn MimeDecoder> = Box::new(Base64Decoder::new());
    let mut encoder = EncoderFilter::new(encoder);
    let mut decoder = DecoderFilter::new(decoder);
    assert_eq!(encoder.encoding(), ContentEncoding::Base64);
    assert_eq!(decoder.encoding(), ContentEncoding::Base64);
    let encoded = run(&mut encoder, b"abc", 1);
    assert_eq!(encoded, b"YWJj\r\n");
    assert_eq!(run(&mut decoder, &encoded, 1), b"abc");
    encoder.reset();
    decoder.reset();
    assert_eq!(run(&mut encoder, b"C", 1), b"Qw==\r\n");
    assert_eq!(run(&mut decoder, b"Qw==", 1), b"C");

    let mut filter: Box<dyn MimeFilter> = Box::new(Dos2UnixFilter::new());
    assert_eq!(run(&mut filter, b"a\r\nb\r", 1), b"a\nb\r");
    MimeFilter::reset(&mut filter);
    assert_eq!(run(&mut filter, b"\r\n", 1), b"\n");
}

const ENCODINGS: [ContentEncoding; 7] = [
    ContentEncoding::Default,
    ContentEncoding::SevenBit,
    ContentEncoding::EightBit,
    ContentEncoding::Binary,
    ContentEncoding::Base64,
    ContentEncoding::QuotedPrintable,
    ContentEncoding::UuEncode,
];

#[test]
fn mimekit_encoder_factory_vectors_and_newline_options() {
    let cases: &[(ContentEncoding, &[u8])] = &[
        (ContentEncoding::Base64, b"YWJj\r\n"),
        (ContentEncoding::QuotedPrintable, b"abc=\r\n"),
        (ContentEncoding::UuEncode, b"#86)C\r\n`\r\n"),
        (ContentEncoding::Default, b"abc"),
        (ContentEncoding::SevenBit, b"abc"),
        (ContentEncoding::EightBit, b"abc"),
        (ContentEncoding::Binary, b"abc"),
    ];
    for &(encoding, dos) in cases {
        assert_eq!(run(&mut encoder_filter(encoding), b"abc", 1), dos);
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let expected = if format == NewLineFormat::Unix {
                dos.iter().copied().filter(|&byte| byte != b'\r').collect()
            } else {
                dos.to_vec()
            };
            let mut filter = encoder_filter_with_options(encoding, 78, format).unwrap();
            assert_eq!(run(&mut filter, b"abc", 1), expected);
            filter.reset();
            assert_eq!(run(&mut filter, b"abc", 2), expected);
        }
    }
}

#[test]
fn factory_validation_order_and_ignored_widths_match_mimekit() {
    for encoding in ENCODINGS {
        for width in [0, 59, 60, 76, 998, 999, usize::MAX] {
            let error = encoder_filter_with_options(encoding, width, NewLineFormat::Mixed)
                .err()
                .unwrap();
            assert_eq!(
                error,
                EncoderOptionsError::InvalidNewLineFormat(NewLineFormat::Mixed),
            );
            for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
                let result = encoder_filter_with_options(encoding, width, format);
                if matches!(
                    encoding,
                    ContentEncoding::Base64 | ContentEncoding::QuotedPrintable
                ) && !(60..=998).contains(&width)
                {
                    let error = result.err().unwrap();
                    match error {
                        EncoderOptionsError::InvalidLineLength(error) => {
                            assert_eq!(error.requested(), width);
                        }
                        other => panic!("unexpected error: {other}"),
                    }
                } else {
                    assert!(result.is_ok());
                }
            }
        }
    }
}

#[test]
fn encoder_factory_honors_effective_line_widths() {
    let input = vec![b'a'; 300];
    for encoding in [ContentEncoding::Base64, ContentEncoding::QuotedPrintable] {
        for width in [60, 61, 75, 76, 77, 998] {
            let effective = if encoding == ContentEncoding::Base64 {
                width.min(76) / 4 * 4
            } else {
                width.min(76)
            };
            for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
                let mut filter = encoder_filter_with_options(encoding, width, format).unwrap();
                let output = run(&mut filter, &input, 7);
                let lines: Vec<_> = output
                    .split(|&byte| byte == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(|line| {
                        if format == NewLineFormat::Dos {
                            line.strip_suffix(b"\r").unwrap()
                        } else {
                            assert!(!line.contains(&b'\r'));
                            line
                        }
                    })
                    .collect();
                assert_eq!(lines[0].len(), effective);
                assert!(lines.iter().all(|line| line.len() <= effective));
                assert_eq!(run(&mut decoder_filter(encoding), &output, 3), input);
            }
        }
    }
}

#[test]
fn factories_preserve_binary_data_for_unencoded_variants() {
    let input = b"a\nb\r\n\0\xff";
    for encoding in ENCODINGS[..4].iter().copied() {
        for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
            let mut filter = encoder_filter_with_options(encoding, 0, format).unwrap();
            assert_eq!(run(&mut filter, input, 1), input);
            assert_eq!(run(&mut decoder_filter(encoding), input, 1), input);
        }
    }
}

#[test]
fn decoder_factory_selects_body_mode_and_uu_header_scanning() {
    let mut base64 = decoder_filter(ContentEncoding::Base64);
    assert_eq!(run(&mut base64, b"!YWJj", 1), b"abc");
    base64.reset();
    assert_eq!(run(&mut base64, b"Qw==", 1), b"C");

    let mut qp = decoder_filter(ContentEncoding::QuotedPrintable);
    assert_eq!(run(&mut qp, b"a_b=3F=\r\n", 1), b"a_b?");

    let mut uu = decoder_filter(ContentEncoding::UuEncode);
    assert_eq!(run(&mut uu, b"#86)C\r\n`\r\n", 1), b"");
    uu.reset();
    assert_eq!(
        run(&mut uu, b"begin 644 file\r\n#86)C\r\n`\r\nend\r\n", 1),
        b"abc",
    );
}

#[test]
fn composition_delivers_final_newline_filter_output_before_encoder_finish() {
    let mut newline = Unix2DosFilter::with_ensure_new_line(true);
    let mut encoder = EncoderFilter::new(Base64Encoder::new());
    let mut intermediate = Vec::new();
    let mut output = Vec::new();
    for chunk in b"a\nb\r".chunks(1) {
        newline.filter(chunk, &mut intermediate);
        encoder.filter(&intermediate, &mut output);
        intermediate.clear();
    }
    newline.finish(&mut intermediate);
    assert_eq!(intermediate, b"\n");
    encoder.filter(&intermediate, &mut output);
    encoder.finish(&mut output);
    assert_eq!(output, b"YQ0KYg0K\r\n");

    let mut newline = Dos2UnixFilter::new();
    encoder.reset();
    intermediate.clear();
    output.clear();
    newline.filter(b"a\r", &mut intermediate);
    encoder.filter(&intermediate, &mut output);
    intermediate.clear();
    newline.finish(&mut intermediate);
    assert_eq!(intermediate, b"\r");
    encoder.filter(&intermediate, &mut output);
    encoder.finish(&mut output);
    assert_eq!(output, b"YQ0=\r\n");
}

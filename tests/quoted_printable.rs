// Includes cases adapted from MimeKit's QuotedPrintableEncoderTests and
// QuotedPrintableDecoderTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::encodings::{
    EncoderOptionsError, NewLineFormat, QuotedPrintableDecoder, QuotedPrintableEncoder,
};

fn encode(input: &[u8], width: usize, chunk_size: usize) -> Vec<u8> {
    encode_with_new_line_format(input, width, chunk_size, NewLineFormat::Dos)
}

fn encode_with_new_line_format(
    input: &[u8],
    width: usize,
    chunk_size: usize,
    ending: NewLineFormat,
) -> Vec<u8> {
    let mut encoder = QuotedPrintableEncoder::with_options(width, ending).unwrap();
    let mut output = Vec::new();
    for chunk in input.chunks(chunk_size) {
        encoder.encode(chunk, &mut output);
        encoder.encode(b"", &mut output);
    }
    encoder.finish(&mut output);
    let length = output.len();
    encoder.finish(&mut output);
    assert_eq!(output.len(), length);
    output
}

fn decode(input: &[u8], chunk_size: usize, encoded_word: bool) -> Vec<u8> {
    let mut decoder = if encoded_word {
        QuotedPrintableDecoder::for_encoded_words()
    } else {
        QuotedPrintableDecoder::new()
    };
    let mut output = Vec::new();
    for chunk in input.chunks(chunk_size) {
        decoder.decode(chunk, &mut output);
        decoder.decode(b"", &mut output);
    }
    output
}

const ENCODING_CASES: &[(&[u8], &[u8])] = &[
    (b"", b""),
    (b"hello", b"hello=\r\n"),
    (b"hello=world ", b"hello=3Dworld=20=\r\n"),
    (b"\n", b"\r\n"),
    (b"\r\n", b"\r\n"),
    (b"\r", b"=0D=\r\n"),
    (b"\r\r", b"=0D=0D=\r\n"),
    (b"\r\r\n", b"=0D\r\n"),
    (b"\rX", b"=0DX=\r\n"),
    (b" \r", b" =0D=\r\n"),
    (b" \t\r\n", b" =09\r\n"),
    (b"word\t ", b"word\t=20=\r\n"),
    (b"_?=", b"_?=3D=\r\n"),
    (b"\0\xff\x7f", b"=00=FF=7F=\r\n"),
    (
        b"This line ends with a space \r\nbefore a line break.",
        b"This line ends with a space=20\r\nbefore a line break.=\r\n",
    ),
    (
        b"This line ends with a space \nbefore a line break.",
        b"This line ends with a space=20\r\nbefore a line break.=\r\n",
    ),
    (
        b"This line ends with a space ",
        b"This line ends with a space=20=\r\n",
    ),
];

const DECODING_CASES: &[(&[u8], &[u8])] = &[
    (b"", b""),
    (b"=", b""),
    (b"=a", b""),
    (b"=\r", b""),
    (b"==", b"="),
    (b"===", b"=="),
    (b"=A=", b"=A="),
    (b"=\r=", b"=\r="),
    (b"=a_", b"=a_"),
    (b"=_", b"=_"),
    (b"=00=FF=7f", b"\0\xff\x7f"),
    (b"=e1=e2=E3=E4\r\n", b"\xe1\xe2\xe3\xe4\r\n"),
    (b"=e1=g2=E3=E4\r\n", b"\xe1=g2\xe3\xe4\r\n"),
    (b"=e1=eg=E3=E4\r\n", b"\xe1=eg\xe3\xe4\r\n"),
    (
        b"   =e1 =e2  =E3\t=E4  \t \t    \r\n",
        b"   \xe1 \xe2  \xe3\t\xe4  \t \t    \r\n",
    ),
    (
        b"Soft line=\r\n\tHard line\r\n",
        b"Soft line\tHard line\r\n",
    ),
    (b"Soft line=\n\tHard line\n", b"Soft line\tHard line\n"),
    (
        b"width==\r\n340 height=3d200\r\n",
        b"width=340 height=200\r\n",
    ),
    (
        b"This is an invalid=\rsoft break.",
        b"This is an invalid=\rsoft break.",
    ),
];

#[test]
fn mimekit_encoding_vectors_and_every_split() {
    for &(input, expected) in ENCODING_CASES {
        for size in 1..=input.len().max(1) {
            assert_eq!(encode(input, 76, size), expected);
        }
        for split in 0..=input.len() {
            let mut encoder = QuotedPrintableEncoder::new();
            let mut output = Vec::new();
            encoder.encode(&input[..split], &mut output);
            encoder.encode(b"", &mut output);
            encoder.encode(&input[split..], &mut output);
            encoder.finish(&mut output);
            assert_eq!(output, expected);
        }
    }
}

#[test]
fn lf_encoding_vectors_and_every_split() {
    for &(input, crlf_expected) in ENCODING_CASES {
        let expected: Vec<u8> = crlf_expected
            .iter()
            .copied()
            .filter(|&b| b != b'\r')
            .collect();
        for split in 0..=input.len() {
            let mut encoder =
                QuotedPrintableEncoder::with_new_line_format(NewLineFormat::Unix).unwrap();
            let mut output = Vec::new();
            encoder.encode(&input[..split], &mut output);
            encoder.encode(b"", &mut output);
            encoder.encode(&input[split..], &mut output);
            encoder.finish(&mut output);
            assert_eq!(output, expected);
            assert_eq!(
                decode(&output, 1, false),
                canonicalize_line_breaks(input, NewLineFormat::Unix)
            );
        }
    }
}

#[test]
fn selected_new_line_formats_match_mimekit() {
    let input = format!("{} \r\nb\nc\rd\t", "a".repeat(76));
    for (ending, newline) in [(NewLineFormat::Dos, "\r\n"), (NewLineFormat::Unix, "\n")] {
        let expected = format!(
            "{}={newline}a=20{newline}b{newline}c=0Dd=09={newline}",
            "a".repeat(75)
        );
        for chunk_size in [1, 75, 76, 77, input.len()] {
            assert_eq!(
                encode_with_new_line_format(input.as_bytes(), 76, chunk_size, ending),
                expected.as_bytes()
            );
        }
        assert_eq!(
            decode(expected.as_bytes(), 1, false),
            format!("{} {newline}b{newline}c\rd\t", "a".repeat(76)).as_bytes()
        );
    }
}

#[test]
fn configurable_new_line_formats_preserve_stream_state_and_configuration() {
    assert_eq!(
        QuotedPrintableEncoder::new().new_line_format(),
        NewLineFormat::Dos
    );
    assert_eq!(
        QuotedPrintableEncoder::default().new_line_format(),
        NewLineFormat::Dos
    );
    assert_eq!(
        QuotedPrintableEncoder::with_line_length(60)
            .unwrap()
            .new_line_format(),
        NewLineFormat::Dos
    );
    let input = [
        vec![b'a'; 73],
        b"\xff \r\nhello\nworld\t\r\r\ntrailing \r".to_vec(),
    ]
    .concat();
    for ending in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut encoder = QuotedPrintableEncoder::with_new_line_format(ending).unwrap();
        assert_eq!(encoder.line_length(), 76);
        assert_eq!(encoder.new_line_format(), ending);
        let mut output = Vec::new();
        encoder.finish(&mut output);
        assert!(output.is_empty());
        for width in 60..=76 {
            let expected = encode_with_new_line_format(&input, width, 4096, ending);
            for split in 0..=input.len() {
                let mut encoder = QuotedPrintableEncoder::with_options(width, ending).unwrap();
                let mut prefix = b"prefix".to_vec();
                encoder.encode(&input[..split], &mut prefix);
                let mut cloned = encoder.clone();
                for encoder in [&mut encoder, &mut cloned] {
                    let mut output = prefix.clone();
                    encoder.encode(&input[split..], &mut output);
                    encoder.finish(&mut output);
                    assert_eq!(&output[..6], b"prefix");
                    assert_eq!(&output[6..], expected);
                    let length = output.len();
                    encoder.finish(&mut output);
                    assert_eq!(output.len(), length);
                    encoder.encode(b"discard\r", &mut output);
                    encoder.reset();
                    output.clear();
                    encoder.encode(&input, &mut output);
                    encoder.finish(&mut output);
                    assert_eq!(output, expected);
                    assert_eq!(encoder.new_line_format(), ending);
                    assert_eq!(encoder.line_length(), width);
                }
            }
        }
        for width in [0, 59, 999, usize::MAX] {
            assert_eq!(
                QuotedPrintableEncoder::with_options(width, ending).unwrap_err(),
                EncoderOptionsError::InvalidLineLength(
                    QuotedPrintableEncoder::with_line_length(width).unwrap_err()
                )
            );
        }
        assert_eq!(
            QuotedPrintableEncoder::with_options(998, ending)
                .unwrap()
                .line_length(),
            76
        );
    }
}

#[test]
fn mixed_new_line_format_is_rejected() {
    let expected = EncoderOptionsError::InvalidNewLineFormat(NewLineFormat::Mixed);
    assert_eq!(
        QuotedPrintableEncoder::with_new_line_format(NewLineFormat::Mixed).unwrap_err(),
        expected
    );
    for width in [0, 59, 60, 76, 998, 999, usize::MAX] {
        assert_eq!(
            QuotedPrintableEncoder::with_options(width, NewLineFormat::Mixed).unwrap_err(),
            expected
        );
    }
}

#[test]
fn mixed_newlines_normalize_to_crlf_across_every_split() {
    let input = b"a\nb\r\nc\rd\r\r\n\n";
    let expected = b"a\r\nb\r\nc=0Dd=0D\r\n\r\n";
    let decoded = b"a\r\nb\r\nc\rd\r\r\n\r\n";
    for split in 0..=input.len() {
        let mut encoder = QuotedPrintableEncoder::new();
        let mut output = Vec::new();
        encoder.encode(&input[..split], &mut output);
        encoder.encode(b"", &mut output);
        encoder.encode(&input[split..], &mut output);
        encoder.finish(&mut output);
        assert_eq!(output, expected);
        assert_eq!(decode(&output, 1, false), decoded);
    }
}

#[test]
fn mimekit_decoding_vectors_and_every_split() {
    for &(input, expected) in DECODING_CASES {
        for size in 1..=input.len().max(1) {
            assert_eq!(decode(input, size, false), expected);
        }
        for split in 0..=input.len() {
            let mut decoder = QuotedPrintableDecoder::new();
            let mut output = Vec::new();
            decoder.decode(&input[..split], &mut output);
            decoder.decode(b"", &mut output);
            decoder.decode(&input[split..], &mut output);
            assert_eq!(output, expected);
        }
    }
}

#[test]
fn line_length_validation_and_normalization() {
    for length in [0, 1, 59, 999, usize::MAX] {
        let error = QuotedPrintableEncoder::with_line_length(length).unwrap_err();
        assert_eq!(error.requested(), length);
        assert!(error.to_string().contains(&length.to_string()));
    }
    for length in 60..=998 {
        let encoder = QuotedPrintableEncoder::with_line_length(length).unwrap();
        assert_eq!(encoder.line_length(), length.min(76));
    }
    assert_eq!(QuotedPrintableEncoder::default().line_length(), 76);
}

#[test]
fn every_single_octet_has_the_expected_encoding() {
    for byte in 0..=255u8 {
        let expected = match byte {
            b'\n' => b"\r\n".to_vec(),
            b'!'..=b'<' | b'>'..=b'~' => vec![byte, b'=', b'\r', b'\n'],
            _ => format!("={byte:02X}=\r\n").into_bytes(),
        };
        assert_eq!(encode(&[byte], 76, 1), expected, "byte={byte}");
        assert_eq!(
            decode(&expected, 1, false),
            if byte == b'\n' {
                b"\r\n"
            } else {
                std::slice::from_ref(&byte)
            }
        );
    }
}

#[test]
fn every_hex_pair_and_invalid_second_digit() {
    for byte in 0..=255u8 {
        for encoded in [format!("={byte:02X}"), format!("={byte:02x}")] {
            for size in 1..=3 {
                assert_eq!(decode(encoded.as_bytes(), size, false), [byte]);
            }
        }
    }
    for first in b"0123456789ABCDEFabcdef" {
        for second in 0..=255u8 {
            if second.is_ascii_hexdigit() {
                continue;
            }
            let input = [b'=', *first, second];
            for mode in [false, true] {
                for size in 1..=3 {
                    assert_eq!(decode(&input, size, mode), input);
                }
            }
        }
    }
}

#[test]
fn equal_sign_and_soft_break_transitions_for_every_byte() {
    for byte in 0..=255u8 {
        if !byte.is_ascii_hexdigit() {
            let expected = match byte {
                b'=' => vec![b'='],
                b'\r' | b'\n' => Vec::new(),
                _ => vec![b'=', byte],
            };
            for mode in [false, true] {
                assert_eq!(decode(&[b'=', byte], 1, mode), expected);
                assert_eq!(decode(&[b'=', byte], 2, mode), expected);
            }
        }
        let expected = if byte == b'\n' {
            Vec::new()
        } else {
            vec![b'=', b'\r', byte]
        };
        for mode in [false, true] {
            for size in 1..=3 {
                assert_eq!(decode(&[b'=', b'\r', byte], size, mode), expected);
            }
        }
    }
}

#[test]
fn mimekit_equal_sign_at_column_76() {
    let input = b"<table style=\"width:100%;\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tr><td style=\"width:100%;text-align:center;background-color:;\" bgcolor=\"\">Test</td></tr><table>";
    let expected = b"<table style=3D\"width:100%;\" cellpadding=3D\"0\" cellspacing=3D\"0\" border=3D\"=\r\n0\"><tr><td style=3D\"width:100%;text-align:center;background-color:;\" bgcolo=\r\nr=3D\"\">Test</td></tr><table>=\r\n";
    for size in [1, 2, 16, 75, 76, 77, 4096] {
        assert_eq!(encode(input, 76, size), expected);
        assert_eq!(decode(expected, size, false), input);
    }
}

#[test]
fn exact_wrapping_at_hard_breaks_and_finish() {
    for width in 60..=76 {
        let prefix = vec![b'a'; width - 2];
        for newline in [b"\n".as_slice(), b"\r\n".as_slice()] {
            for (suffix, encoded_suffix) in [
                (b"\xe9b".as_slice(), b"=E9b".as_slice()),
                (b"\xe9", b"=E9"),
                (b" ", b"=20"),
                (b"\t", b"=09"),
                (b"\r\xe9", b"=0D=E9"),
            ] {
                let input = [prefix.as_slice(), suffix, newline].concat();
                let expected = [prefix.as_slice(), b"=\r\n", encoded_suffix, b"\r\n"].concat();
                for size in [1, 2, 16, 4096] {
                    assert_eq!(encode(&input, width, size), expected);
                }
            }
        }
        let prefix = vec![b'a'; width - 3];
        let input = [prefix.as_slice(), b"\xe9\r\n", &prefix, b"\xe9\r"].concat();
        let expected = [prefix.as_slice(), b"=E9\r\n", &prefix, b"=\r\n=E9=0D=\r\n"].concat();
        for size in [1, 2, 16, 4096] {
            assert_eq!(encode(&input, width, size), expected);
            assert_eq!(
                encode(&[prefix.as_slice(), b"\xe9"].concat(), width, size),
                [prefix.as_slice(), b"=\r\n=E9=\r\n"].concat()
            );
        }
    }
}

fn canonicalize_line_breaks(input: &[u8], ending: NewLineFormat) -> Vec<u8> {
    let mut output = Vec::new();
    for (index, &byte) in input.iter().enumerate() {
        if byte == b'\r' && input.get(index + 1) == Some(&b'\n') {
            continue;
        }
        if byte == b'\n' && ending == NewLineFormat::Dos {
            output.push(b'\r');
        }
        output.push(byte);
    }
    output
}

#[test]
fn all_octets_round_trip_with_bounded_lines_and_arbitrary_chunks() {
    let mut state = 0xf135_7ae1u32;
    let mut input: Vec<u8> = (0..=255).cycle().take(512).collect();
    input.extend((0..16384).map(|_| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as u8
    }));
    for ending in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let expected = canonicalize_line_breaks(&input, ending);
        for width in 60..=76 {
            let encoded = encode_with_new_line_format(&input, width, input.len(), ending);
            if ending == NewLineFormat::Unix {
                let expected_lf: Vec<u8> = encode(&input, width, input.len())
                    .into_iter()
                    .filter(|&b| b != b'\r')
                    .collect();
                assert_eq!(encoded, expected_lf);
            }
            assert!(encoded.is_ascii());
            for line in encoded.split_inclusive(|&b| b == b'\n') {
                let newline = if ending == NewLineFormat::Dos {
                    b"\r\n".as_slice()
                } else {
                    b"\n"
                };
                let line = line.strip_suffix(newline).expect("selected line ending");
                assert!(!line.contains(&b'\r'));
                assert!(line.len() <= width, "width={width}, length={}", line.len());
                assert!(!line.ends_with(b" ") && !line.ends_with(b"\t"));
            }
            for size in [1, 2, 3, 15, 16, 17, 33, 77, 4096] {
                assert_eq!(
                    encode_with_new_line_format(&input, width, size, ending),
                    encoded
                );
                assert_eq!(decode(&encoded, size, false), expected);
            }
        }
    }
}

#[test]
fn pass_through_run_boundaries() {
    // Place escapes around the scalar scan length (16) and much farther into
    // the input, exercising both the per-byte and search-and-copy paths.
    for length in [0, 1, 15, 16, 17, 31, 32, 33, 100, 1000] {
        let mut input = Vec::new();
        let mut expected = Vec::new();
        for (escape, plain) in [
            (b"=C3=A9".as_slice(), b"\xc3\xa9".as_slice()),
            (b"=\r\n", b""),
            (b"=\n", b""),
            (b"==3D=g=\rX=", b"===g=\rX"),
        ] {
            input.extend(vec![b'x'; length]);
            input.extend_from_slice(escape);
            expected.extend(vec![b'x'; length]);
            expected.extend_from_slice(plain);
        }
        for size in [1, 2, 3, 15, 16, 17, 33, 77, input.len()] {
            assert_eq!(decode(&input, size, false), expected);
        }
    }
}

#[test]
fn encoded_word_mode_and_malformed_underscores() {
    let input = b"hello_world=5F=C3=A9_=_=a_=\r_";
    let expected = b"hello world_\xc3\xa9 =_=a_=\r_";
    for size in 1..=input.len() {
        assert_eq!(decode(input, size, true), expected);
    }
    assert_eq!(decode(b"_=5F_", 1, false), b"___");
    assert_eq!(decode(b"_=5F_", 1, true), b" _ ");
    let input = [
        vec![b'_'; 64],
        b"=C3=A9".to_vec(),
        vec![b'a'; 64],
        b"_b".to_vec(),
    ]
    .concat();
    let expected = [
        vec![b' '; 64],
        b"\xc3\xa9".to_vec(),
        vec![b'a'; 64],
        b" b".to_vec(),
    ]
    .concat();
    for size in [1, 16, 17, 4096] {
        assert_eq!(decode(&input, size, true), expected);
    }
}

#[test]
fn clone_reset_finish_and_buffer_reuse() {
    for prefix in [b"".as_slice(), b"a", b"a ", b"a\r", b"\r", b"\t"] {
        let mut encoder = QuotedPrintableEncoder::with_line_length(61).unwrap();
        let mut head = b"prefix".to_vec();
        encoder.encode(prefix, &mut head);
        let mut clone = encoder.clone();
        let mut suffix = Vec::new();
        let mut cloned_suffix = Vec::new();
        encoder.encode(b"\n=\xff ", &mut suffix);
        clone.encode(b"\n=\xff ", &mut cloned_suffix);
        encoder.finish(&mut suffix);
        clone.finish(&mut cloned_suffix);
        assert_eq!(suffix, cloned_suffix);
        assert_eq!(
            [head, suffix].concat(),
            [
                b"prefix".as_slice(),
                &encode(&[prefix, b"\n=\xff "].concat(), 61, 4096)
            ]
            .concat()
        );
        encoder.encode(b"discard\r", &mut cloned_suffix);
        encoder.reset();
        cloned_suffix.clear();
        encoder.encode(b"x", &mut cloned_suffix);
        encoder.finish(&mut cloned_suffix);
        assert_eq!(cloned_suffix, b"x=\r\n");
        assert_eq!(encoder.line_length(), 61);
    }
    for mode in [false, true] {
        for prefix in [b"".as_slice(), b"=", b"=4", b"=\r"] {
            let mut decoder = if mode {
                QuotedPrintableDecoder::for_encoded_words()
            } else {
                QuotedPrintableDecoder::default()
            };
            let mut output = b"prefix".to_vec();
            decoder.decode(prefix, &mut output);
            assert_eq!(output, b"prefix");
            let mut clone = decoder.clone();
            output.clear();
            let mut other = Vec::new();
            decoder.decode(b"1_", &mut output);
            clone.decode(b"1_", &mut other);
            assert_eq!(output, other);
            assert_eq!(output, decode(&[prefix, b"1_"].concat(), 4096, mode));
            decoder.decode(b"=\r", &mut output);
            decoder.reset();
            output.clear();
            decoder.decode(b"_", &mut output);
            assert_eq!(output, if mode { b" " } else { b"_" });
        }
    }
}

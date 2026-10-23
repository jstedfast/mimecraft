// Includes cases adapted from MimeKit's Base64EncoderTests and Base64DecoderTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::encodings::{Base64Decoder, Base64Encoder, EncoderOptionsError, NewLineFormat};

fn encode(input: &[u8], line_length: usize, chunk_size: usize) -> Vec<u8> {
    let mut encoder = Base64Encoder::with_line_length(line_length).unwrap();
    let mut output = Vec::new();
    for chunk in input.chunks(chunk_size) {
        encoder.encode(chunk, &mut output);
        encoder.encode(b"", &mut output);
    }
    encoder.finish(&mut output);
    output
}

fn decode(input: &[u8], chunk_size: usize) -> Vec<u8> {
    let mut decoder = Base64Decoder::new();
    let mut output = Vec::new();
    for chunk in input.chunks(chunk_size) {
        decoder.decode(chunk, &mut output);
        decoder.decode(b"", &mut output);
    }
    output
}

#[test]
fn known_encoding_vectors() {
    for (plain, expected) in [
        ("", ""),
        ("f", "Zg==\r\n"),
        ("fo", "Zm8=\r\n"),
        ("foo", "Zm9v\r\n"),
        ("foob", "Zm9vYg==\r\n"),
        ("fooba", "Zm9vYmE=\r\n"),
        ("foobar", "Zm9vYmFy\r\n"),
    ] {
        for chunk in 1..=8 {
            assert_eq!(encode(plain.as_bytes(), 76, chunk), expected.as_bytes());
            assert_eq!(decode(expected.as_bytes(), chunk), plain.as_bytes());
        }
    }
}

#[test]
fn configurable_new_line_formats_preserve_stream_state_and_configuration() {
    assert_eq!(NewLineFormat::default(), NewLineFormat::Dos);
    assert_eq!(Base64Encoder::new().new_line_format(), NewLineFormat::Dos);
    assert_eq!(
        Base64Encoder::default().new_line_format(),
        NewLineFormat::Dos
    );
    assert_eq!(
        Base64Encoder::with_line_length(60)
            .unwrap()
            .new_line_format(),
        NewLineFormat::Dos
    );
    let input: Vec<u8> = (0..=255).cycle().take(513).collect();
    for ending in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut encoder = Base64Encoder::with_new_line_format(ending).unwrap();
        assert_eq!(encoder.line_length(), 76);
        assert_eq!(encoder.new_line_format(), ending);
        let mut output = Vec::new();
        encoder.finish(&mut output);
        assert!(output.is_empty());
        encoder.encode(b"f", &mut output);
        encoder.finish(&mut output);
        assert_eq!(
            output,
            if ending == NewLineFormat::Unix {
                b"Zg==\n".as_slice()
            } else {
                b"Zg==\r\n"
            }
        );
        for width in [60, 63, 64, 72, 75, 76, 998] {
            let expected: Vec<u8> = encode(&input, width, 4096)
                .into_iter()
                .filter(|&b| ending == NewLineFormat::Dos || b != b'\r')
                .collect();
            for split in 0..=input.len() {
                let mut encoder = Base64Encoder::with_options(width, ending).unwrap();
                let mut prefix = b"prefix".to_vec();
                encoder.encode(&input[..split], &mut prefix);
                let mut cloned = encoder.clone();
                for encoder in [&mut encoder, &mut cloned] {
                    assert_eq!(encoder.new_line_format(), ending);
                    let mut output = prefix.clone();
                    encoder.encode(&input[split..], &mut output);
                    encoder.finish(&mut output);
                    assert_eq!(&output[..6], b"prefix");
                    assert_eq!(&output[6..], expected);
                    let length = output.len();
                    encoder.finish(&mut output);
                    assert_eq!(output.len(), length);
                    encoder.encode(b"discard", &mut output);
                    encoder.reset();
                    output.clear();
                    encoder.encode(&input, &mut output);
                    encoder.finish(&mut output);
                    assert_eq!(output, expected);
                    assert_eq!(encoder.new_line_format(), ending);
                    assert_eq!(encoder.line_length(), width.min(76) / 4 * 4);
                }
            }
        }
        for width in [0, 59, 999, usize::MAX] {
            assert_eq!(
                Base64Encoder::with_options(width, ending).unwrap_err(),
                EncoderOptionsError::InvalidLineLength(
                    Base64Encoder::with_line_length(width).unwrap_err()
                )
            );
        }
    }
}

#[test]
fn mixed_new_line_format_is_rejected() {
    let expected = EncoderOptionsError::InvalidNewLineFormat(NewLineFormat::Mixed);
    assert_eq!(
        Base64Encoder::with_new_line_format(NewLineFormat::Mixed).unwrap_err(),
        expected
    );
    for width in [0, 59, 60, 76, 998, 999, usize::MAX] {
        assert_eq!(
            Base64Encoder::with_options(width, NewLineFormat::Mixed).unwrap_err(),
            expected
        );
    }
    assert_eq!(
        expected.to_string(),
        "new-line format must be Dos or Unix, got Mixed"
    );
    assert!(std::error::Error::source(&expected).is_none());
    let error = Base64Encoder::with_options(59, NewLineFormat::Dos).unwrap_err();
    let length_error = Base64Encoder::with_line_length(59).unwrap_err();
    assert_eq!(error.to_string(), length_error.to_string());
    assert_eq!(
        std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<mimecraft::encodings::InvalidLineLength>(),
        Some(&length_error)
    );
}

#[test]
fn crlf_output_preserves_payload_and_accepts_legacy_lf_wrapping() {
    let input = b"a\nb\r\nc\rd";
    for chunk in 1..=16 {
        assert_eq!(encode(input, 76, chunk), b"YQpiDQpjDWQ=\r\n");
        assert_eq!(decode(b"YQpi\r\nDQpj\r\nDWQ=\r\n", chunk), input);
        assert_eq!(decode(b"YQpi\nDQpj\nDWQ=\n", chunk), input);
    }
}

#[test]
fn mimekit_decoding_vectors() {
    let cases = [
        (
            "VGhpcyBpcyB0aGUgcGxhaW4gdGV4dCBtZXNzYWdlIQ==",
            "This is the plain text message!",
        ),
        (
            "VGhpcyBpcyBhIHRleHQgd2hpY2ggaGFzIHRvIGJlIHBhZGRlZCBvbmNlLi4=",
            "This is a text which has to be padded once..",
        ),
        (
            "VGhpcyBpcyBhIHRleHQgd2hpY2ggaGFzIHRvIGJlIHBhZGRlZCB0d2ljZQ==",
            "This is a text which has to be padded twice",
        ),
        (
            "VGhpcyBpcyBhIHRleHQgd2hpY2ggd2lsbCBub3QgYmUgcGFkZGVk",
            "This is a text which will not be padded",
        ),
        (
            " &% VGhp\r\ncyBp\r\ncyB0aGUgcGxhaW4g  \tdGV4dCBtZ?!XNzY*WdlIQ==",
            "This is the plain text message!",
        ),
    ];
    for (encoded, expected) in cases {
        for chunk in 1..=encoded.len() {
            assert_eq!(decode(encoded.as_bytes(), chunk), expected.as_bytes());
        }
    }
    for count in 1..=4 {
        let encoded = format!("{}{}", cases[0].0, "=".repeat(count));
        for chunk in 1..=encoded.len() {
            assert_eq!(decode(encoded.as_bytes(), chunk), cases[0].1.as_bytes());
        }
    }
}

#[test]
fn concatenated_padded_blocks() {
    let input = b"VGhpcyBpcyB0aGUgcGF5bG9hZCBvZiB0aGUgZmlyc3QgYmFzZTY0LWVuY29kZWQgYmxvY2sgb2Yg\r\ndGV4dC4=\r\nQW5kIHRoaXMgaXMgdGhlIHBheWxvYWQgb2YgdGhlIHNlY29uZCBiYXNlNjQtZW5jb2RlZCBibG9j\r\nayBvZiB0ZXh0Lg==\r\n";
    let expected = b"This is the payload of the first base64-encoded block of text.And this is the payload of the second base64-encoded block of text.";
    for chunk in 1..=input.len() {
        assert_eq!(decode(input, chunk), expected);
    }
}

#[test]
fn line_length_validation_and_normalization() {
    for length in [0, 1, 59, 999, usize::MAX] {
        let error = Base64Encoder::with_line_length(length).unwrap_err();
        assert_eq!(error.requested(), length);
        assert!(error.to_string().contains(&length.to_string()));
    }
    for requested in 60..=998 {
        let encoder = Base64Encoder::with_line_length(requested).unwrap();
        assert_eq!(encoder.line_length(), requested.min(76) / 4 * 4);
    }
    assert_eq!(Base64Encoder::new().line_length(), 76);
}

#[test]
fn exact_wrapping_padding_and_final_newline() {
    for requested in [60, 61, 63, 64, 72, 75, 76, 77, 998] {
        let width = requested.min(76) / 4 * 4;
        for length in 0..=200 {
            let input = vec![b'a'; length];
            let mut unwrapped = "YWFh".repeat(length / 3);
            unwrapped.push_str(match length % 3 {
                1 => "YQ==",
                2 => "YWE=",
                _ => "",
            });
            let mut expected = Vec::new();
            for line in unwrapped.as_bytes().chunks(width) {
                expected.extend_from_slice(line);
                expected.extend_from_slice(b"\r\n");
            }
            for chunk in [1, 2, 3, 15, 16, 17, 55, 56, 57, 58, 4096] {
                assert_eq!(encode(&input, requested, chunk), expected);
            }
        }
    }
}

#[test]
fn all_byte_values_match_known_vector() {
    let input: Vec<u8> = (0..=255).collect();
    let expected = concat!(
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCU",
        "mJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0",
        "xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3Bxc",
        "nN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeY",
        "mZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6",
        "/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5O",
        "Xm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w==",
    );
    let mut wrapped = Vec::new();
    for line in expected.as_bytes().chunks(76) {
        wrapped.extend_from_slice(line);
        wrapped.extend_from_slice(b"\r\n");
    }
    for chunk in 1..=input.len() {
        assert_eq!(encode(&input, 76, chunk), wrapped);
        assert_eq!(decode(&wrapped, chunk), input);
    }
}

#[test]
fn every_two_way_input_split() {
    let input: Vec<u8> = (0..=255).cycle().take(513).collect();
    let expected = encode(&input, 76, input.len());
    for split in 0..=input.len() {
        let mut encoder = Base64Encoder::new();
        let mut output = Vec::new();
        encoder.encode(&input[..split], &mut output);
        encoder.encode(&input[split..], &mut output);
        encoder.finish(&mut output);
        assert_eq!(output, expected, "encode split {split}");
    }
    for split in 0..=expected.len() {
        let mut decoder = Base64Decoder::new();
        let mut output = Vec::new();
        decoder.decode(&expected[..split], &mut output);
        decoder.decode(&expected[split..], &mut output);
        assert_eq!(output, input, "decode split {split}");
    }
}

#[test]
fn encoder_clone_reset_finish_and_buffer_reuse() {
    for prefix_len in 0..=60 {
        let mut encoder = Base64Encoder::with_line_length(60).unwrap();
        let mut prefix = Vec::new();
        encoder.encode(&vec![b'a'; prefix_len], &mut prefix);
        let mut cloned = encoder.clone();
        let mut left = prefix.clone();
        let mut right = prefix;
        encoder.encode(b"bcde", &mut left);
        cloned.encode(b"bcde", &mut right);
        encoder.finish(&mut left);
        cloned.finish(&mut right);
        assert_eq!(left, right);
        let length = left.len();
        encoder.finish(&mut left);
        assert_eq!(left.len(), length);

        encoder.encode(b"x", &mut left);
        encoder.reset();
        assert_eq!(encoder.line_length(), 60);
        encoder.encode(b"f", &mut left);
        encoder.finish(&mut left);
        assert_eq!(&left[length..], b"Zg==\r\n");
    }

    let mut encoder = Base64Encoder::new();
    let mut output = b"prefix".to_vec();
    encoder.encode(b"foo", &mut output);
    assert_eq!(output, b"prefixZm9v");
    output.clear();
    encoder.encode(b"bar", &mut output);
    encoder.finish(&mut output);
    assert_eq!(output, b"YmFy\r\n");
    output.clear();
    encoder.encode(b"f", &mut output);
    encoder.finish(&mut output);
    assert_eq!(output, b"Zg==\r\n");
}

#[test]
fn decoder_clone_reset_and_partial_quartets() {
    for split in 0..=8 {
        let mut decoder = Base64Decoder::new();
        let mut output = b"prefix".to_vec();
        decoder.decode(&b"Zm9vYg=="[..split], &mut output);
        let mut cloned = decoder.clone();
        let mut other = output.clone();
        decoder.decode(&b"Zm9vYg=="[split..], &mut output);
        cloned.decode(&b"Zm9vYg=="[split..], &mut other);
        assert_eq!(output, b"prefixfoob");
        assert_eq!(other, output);
    }
    for incomplete in [b"Z".as_slice(), b"Zg", b"Zg="] {
        let mut decoder = Base64Decoder::new();
        let mut output = Vec::new();
        decoder.decode(incomplete, &mut output);
        assert!(output.is_empty());
        decoder.reset();
        decoder.decode(b"Zm9v", &mut output);
        assert_eq!(output, b"foo");
    }
    let mut decoder = Base64Decoder::new();
    let mut output = Vec::new();
    decoder.decode(b"Zm9vY", &mut output);
    assert_eq!(output, b"foo");
    output.clear();
    decoder.decode(b"g==", &mut output);
    assert_eq!(output, b"b");
}

#[test]
fn malformed_padding_preserves_mimekit_behavior() {
    for (encoded, expected) in [
        (b"=AAA".as_slice(), b"\0\0\0".as_slice()),
        (b"A=AA", b""),
        (b"AA=A", b"\0"),
        (b"AAA=", b"\0\0"),
        (b"====", b""),
        (b"Zg======Zm8=", b"ffo"),
    ] {
        for chunk in 1..=encoded.len() {
            assert_eq!(decode(encoded, chunk), expected);
        }
    }
}

#[test]
fn every_non_alphabet_byte_is_ignored_at_every_quartet_position() {
    for byte in 0..=255u8 {
        if byte.is_ascii_alphanumeric() || b"+/=".contains(&byte) {
            continue;
        }
        for position in 0..=4 {
            let mut input = b"Zm9v".to_vec();
            input.insert(position, byte);
            for chunk in 1..=input.len() {
                assert_eq!(decode(&input, chunk), b"foo");
            }
        }
    }
}

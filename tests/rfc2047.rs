// Includes cases adapted from MimeKit's Rfc2047QuotedPrintableEncoderTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::encodings::{
    Base64Decoder, Base64Encoder, QEncodeMode, QuotedPrintableDecoder, Rfc2047Base64Encoder,
    Rfc2047QuotedPrintableEncoder,
};

fn reference_base64(input: &[u8]) -> Vec<u8> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = Vec::new();
    for group in input.chunks(3) {
        let word = group
            .iter()
            .fold(0u32, |word, &byte| word << 8 | u32::from(byte))
            << (8 * (3 - group.len()));
        for index in 0..4 {
            output.push(if index <= group.len() {
                alphabet[((word >> (18 - index * 6)) & 63) as usize]
            } else {
                b'='
            });
        }
    }
    output
}

fn reference_q(input: &[u8], mode: QEncodeMode) -> Vec<u8> {
    let mut output = Vec::new();
    for &byte in input {
        let safe = match mode {
            QEncodeMode::Phrase => byte.is_ascii_alphanumeric() || b"!*+-/".contains(&byte),
            QEncodeMode::Text => {
                (32..127).contains(&byte) && !b"()<>@,;:\"/[]?.=_\\".contains(&byte)
            }
        };
        if byte == b' ' {
            output.push(b'_');
        } else if safe {
            output.push(byte);
        } else {
            output.extend_from_slice(format!("={byte:02X}").as_bytes());
        }
    }
    output
}

#[test]
fn b_payload_vectors_padding_and_no_newlines() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"", b""),
        (b"f", b"Zg=="),
        (b"fo", b"Zm8="),
        (b"foo", b"Zm9v"),
        (b"foob", b"Zm9vYg=="),
        (b"fooba", b"Zm9vYmE="),
        (b"foobar", b"Zm9vYmFy"),
        (b"\xff", b"/w=="),
        (b"\xff\xff", b"//8="),
        (b"\xff\xff\xff", b"////"),
        (b"hello world", b"aGVsbG8gd29ybGQ="),
    ];
    let encoder = Rfc2047Base64Encoder::new();
    for &(input, expected) in cases {
        let mut output = b"prefix".to_vec();
        encoder.encode(input, &mut output);
        encoder.encode(b"", &mut output);
        assert_eq!(&output[..6], b"prefix");
        assert_eq!(&output[6..], expected);
        assert_eq!(reference_base64(input), expected);
    }
}

#[test]
fn b_calls_are_independent_complete_payloads() {
    let encoder = Rfc2047Base64Encoder::new();
    let mut output = Vec::new();
    encoder.encode(b"a", &mut output);
    encoder.encode(b"b", &mut output);
    assert_eq!(output, b"YQ==Yg==");
    let mut decoded = Vec::new();
    Base64Decoder::new().decode(&output, &mut decoded);
    assert_eq!(decoded, b"ab");
    output.clear();
    encoder.encode(b"ab", &mut output);
    assert_eq!(output, b"YWI=");
}

#[test]
fn b_all_bytes_lengths_alignment_and_vector_tails() {
    let data: Vec<u8> = (0..=255).cycle().take(10040).collect();
    let encoder = Rfc2047Base64Encoder::new();
    for length in (0..=256).chain([511, 512, 513, 1023, 4095, 4096, 4097, 10007]) {
        for offset in 0..32 {
            let input = &data[offset..offset + length];
            let expected = reference_base64(input);
            let mut output = Vec::new();
            encoder.encode(input, &mut output);
            assert_eq!(output, expected, "length {length}, offset {offset}");
            assert_eq!(output.len(), length.div_ceil(3) * 4);
            assert!(!output.iter().any(|byte| matches!(byte, b'\r' | b'\n')));
            let mut decoded = Vec::new();
            Base64Decoder::new().decode(&output, &mut decoded);
            assert_eq!(decoded, input);
        }
    }
}

#[test]
fn b_payload_agrees_with_mime_encoder_without_line_breaks() {
    let input: Vec<u8> = (0..=255).cycle().take(10007).collect();
    let mut body = Vec::new();
    let mut encoder = Base64Encoder::new();
    for chunk in input.chunks(7) {
        encoder.encode(chunk, &mut body);
    }
    encoder.finish(&mut body);
    body.retain(|byte| !matches!(byte, b'\r' | b'\n'));
    let mut payload = Vec::new();
    Rfc2047Base64Encoder::new().encode(&input, &mut payload);
    assert_eq!(payload, body);
}

#[test]
fn q_phrase_and_text_source_vectors() {
    let input = b" \t\r\nABCabc123!@#$%^&*()_+`-=[]\\{}|;:'\",./<>?";
    let cases: &[(QEncodeMode, &[u8])] = &[
        (QEncodeMode::Text, b"_=09=0D=0AABCabc123!=40#$%^&*=28=29=5F+`-=3D=5B=5D=5C{}|=3B=3A'=22=2C=2E=2F=3C=3E=3F"),
        (QEncodeMode::Phrase, b"_=09=0D=0AABCabc123!=40=23=24=25=5E=26*=28=29=5F+=60-=3D=5B=5D=5C=7B=7D=7C=3B=3A=27=22=2C=2E/=3C=3E=3F"),
    ];
    for &(mode, expected) in cases {
        let encoder = Rfc2047QuotedPrintableEncoder::new(mode);
        assert_eq!(encoder.mode(), mode);
        assert_eq!(reference_q(input, mode), expected);
        for size in 1..=input.len() + 1 {
            let mut output = b"prefix".to_vec();
            for chunk in input.chunks(size) {
                encoder.encode(chunk, &mut output);
                encoder.encode(b"", &mut output);
            }
            assert_eq!(&output[..6], b"prefix");
            assert_eq!(&output[6..], expected);
            let mut decoder = QuotedPrintableDecoder::for_encoded_words();
            let mut decoded = Vec::new();
            for chunk in expected.chunks(size) {
                decoder.decode(chunk, &mut decoded);
            }
            assert_eq!(decoded, input);
        }
    }
}

#[test]
fn q_all_byte_classifications_and_round_trip() {
    for mode in [QEncodeMode::Phrase, QEncodeMode::Text] {
        let encoder = Rfc2047QuotedPrintableEncoder::new(mode);
        for byte in 0..=255 {
            let mut output = Vec::new();
            encoder.encode(&[byte], &mut output);
            assert_eq!(output, reference_q(&[byte], mode), "{byte}, {mode:?}");
            let mut decoder = QuotedPrintableDecoder::for_encoded_words();
            let mut decoded = Vec::new();
            for chunk in output.chunks(1) {
                decoder.decode(chunk, &mut decoded);
            }
            assert_eq!(decoded, [byte]);
        }
        let input: Vec<u8> = (0..=255).collect();
        for size in [1, 2, 3, 7, 16, 255, 256] {
            let mut output = Vec::new();
            for chunk in input.chunks(size) {
                encoder.encode(chunk, &mut output);
            }
            assert_eq!(output, reference_q(&input, mode));
        }
    }
}

#[test]
fn q_does_not_apply_body_wrapping_or_normalization_rules() {
    let input = [vec![b'a'; 999], b" \t\r\n_\r ?= \xff".to_vec()].concat();
    for mode in [QEncodeMode::Phrase, QEncodeMode::Text] {
        let encoder = Rfc2047QuotedPrintableEncoder::new(mode);
        let mut output = Vec::new();
        encoder.encode(&input, &mut output);
        assert_eq!(&output[..999], vec![b'a'; 999]);
        assert_eq!(&output[999..], b"_=09=0D=0A=5F=0D_=3F=3D_=FF");
        assert!(!output.iter().any(|byte| matches!(byte, b'\r' | b'\n')));
        let mut decoded = Vec::new();
        QuotedPrintableDecoder::for_encoded_words().decode(&output, &mut decoded);
        assert_eq!(decoded, input);
        output.clear();
        encoder.encode(b"", &mut output);
        assert!(output.is_empty());
        encoder.encode(b" ", &mut output);
        assert_eq!(output, b"_");
    }
}

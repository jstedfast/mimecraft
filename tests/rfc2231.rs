// Includes cases adapted from MimeKit's Rfc2231EncoderTests and Rfc2231DecoderTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::encodings::{Rfc2231Decoder, Rfc2231Encoder};

fn reference_encode(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for &byte in input {
        let attr = (33..127).contains(&byte) && !b"*'%()<>@,;:\\\"/[]?=".contains(&byte);
        if attr {
            output.push(byte);
        } else {
            output.extend_from_slice(format!("%{byte:02X}").as_bytes());
        }
    }
    output
}

// Whole-input parser, using standard integer parsing instead of the codec's
// incremental state machine and hex classification helper.
fn reference_decode(mut input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(index) = input.iter().position(|&byte| byte == b'%') {
        output.extend_from_slice(&input[..index]);
        input = &input[index..];
        if input.len() < 3 {
            return output;
        }
        let value = if input[1..3].iter().all(u8::is_ascii_hexdigit) {
            Some(u8::from_str_radix(std::str::from_utf8(&input[1..3]).unwrap(), 16).unwrap())
        } else {
            None
        };
        if let Some(value) = value {
            output.push(value);
        } else {
            output.extend_from_slice(&input[..3]);
        }
        input = &input[3..];
    }
    output.extend_from_slice(input);
    output
}

fn decode(input: &[u8], chunk_size: usize) -> Vec<u8> {
    let mut decoder = Rfc2231Decoder::new();
    let mut output = b"prefix".to_vec();
    for chunk in input.chunks(chunk_size) {
        decoder.decode(chunk, &mut output);
        decoder.decode(b"", &mut output);
    }
    assert_eq!(&output[..6], b"prefix");
    output[6..].to_vec()
}

#[test]
fn mimekit_parameter_vectors() {
    let cases: &[(&[u8], &[u8])] = &[
        (
            b" \t\r\nABCabc123!@#$%^&*()_+`-=[]\\{}|;:'\",./<>?",
            b"%20%09%0D%0AABCabc123!%40#$%25^&%2A%28%29_+`-%3D%5B%5D%5C{}|%3B%3A%27%22%2C.%2F%3C%3E%3F",
        ),
        (
            b"\xed\xe5\xec\xf9 \xef\xe1 \xe9\xec\xe8\xf4\xf0",
            b"%ED%E5%EC%F9%20%EF%E1%20%E9%EC%E8%F4%F0",
        ),
        (b"caf\xc3\xa9.txt", b"caf%C3%A9.txt"),
        (b"a+b c", b"a+b%20c"),
        (b"", b""),
    ];
    for &(input, expected) in cases {
        assert_eq!(reference_encode(input), expected);
        let mut actual = Vec::new();
        Rfc2231Encoder::new().encode(input, &mut actual);
        assert_eq!(actual, expected);
        for size in 1..=expected.len() + 1 {
            assert_eq!(decode(expected, size), input);
        }
    }
    let input =
        b"This should decode: (%ED%E5%EC%F9 %EF%E1 %E9%EC%E8%F4%F0) while %X1%S1%Z1 should not";
    let expected = b"This should decode: (\xed\xe5\xec\xf9 \xef\xe1 \xe9\xec\xe8\xf4\xf0) while %X1%S1%Z1 should not";
    assert_eq!(decode(input, 1), expected);
}

#[test]
fn every_byte_classification_and_round_trip() {
    let input: Vec<u8> = (0..=255).collect();
    let expected = reference_encode(&input);
    for size in [1, 2, 3, 7, 16, 61, 256] {
        let mut actual = b"prefix".to_vec();
        let encoder = Rfc2231Encoder::new();
        for chunk in input.chunks(size) {
            encoder.encode(chunk, &mut actual);
            encoder.encode(b"", &mut actual);
        }
        assert_eq!(&actual[..6], b"prefix");
        assert_eq!(&actual[6..], expected);
        assert_eq!(decode(&expected, size), input);
    }
    for byte in 0..=255 {
        let mut encoded = Vec::new();
        Rfc2231Encoder::new().encode(&[byte], &mut encoded);
        assert_eq!(encoded, reference_encode(&[byte]), "byte {byte}");
        assert_eq!(decode(&encoded, 1), [byte]);
        // Lowercase escapes are accepted too.
        let lowercase = format!("%{byte:02x}");
        assert_eq!(decode(lowercase.as_bytes(), 1), [byte]);
    }
}

#[test]
fn invalid_sequences_are_literal_without_reinterpreting_percent() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"%X1%S1%Z1", b"%X1%S1%Z1"),
        (b"%%41", b"%%41"),
        (b"%4%41", b"%4%41"),
        (b"%%%41", b"%%%41"),
        (b"%+1%-1", b"%+1%-1"),
        (b"%\r\n", b"%\r\n"),
        (b"%\0\xff", b"%\0\xff"),
        (b"a+b_c\r\n", b"a+b_c\r\n"),
        (b"%00%ff%aA%41", b"\0\xff\xaaA"),
        (b"abc%", b"abc"),
        (b"abc%X", b"abc"),
        (b"abc%4", b"abc"),
    ];
    for &(input, expected) in cases {
        assert_eq!(reference_decode(input), expected);
        for size in 1..=input.len() + 1 {
            assert_eq!(decode(input, size), expected, "{input:?}, {size}");
        }
    }
}

#[test]
fn every_escape_pair_and_chunk_boundary() {
    for a in 0..=255 {
        for b in 0..=255 {
            let input = [b'%', a, b, b'%', b'4', b'1'];
            let expected = reference_decode(&input);
            for size in [1, 2, 3, 6] {
                assert_eq!(decode(&input, size), expected, "{a}/{b}, {size}");
            }
        }
    }
}

#[test]
fn incomplete_escapes_wait_even_when_already_invalid() {
    let mut decoder = Rfc2231Decoder::new();
    let mut output = Vec::new();
    decoder.decode(b"%", &mut output);
    decoder.decode(b"", &mut output);
    assert!(output.is_empty());
    decoder.decode(b"X", &mut output);
    decoder.decode(b"", &mut output);
    assert!(output.is_empty());
    decoder.decode(b"Y", &mut output);
    assert_eq!(output, b"%XY");
    output.clear();
    decoder.decode(b"%4", &mut output);
    assert!(output.is_empty());
    decoder.decode(b"1", &mut output);
    assert_eq!(output, b"A");
    for pending in [b"%".as_slice(), b"%4", b"%X"] {
        output.clear();
        decoder.decode(pending, &mut output);
        assert!(output.is_empty());
        decoder.reset();
        decoder.decode(b"41", &mut output);
        assert_eq!(output, b"41");
    }
}

#[test]
fn clone_reset_and_drained_output_at_every_split() {
    let input = b"raw+text%20%ed%E5%00%ff%%41%4%41%\r\n%X";
    let expected = reference_decode(input);
    for split in 0..=input.len() {
        let mut decoder = Rfc2231Decoder::new();
        let mut prefix = Vec::new();
        decoder.decode(&input[..split], &mut prefix);
        let mut clone = decoder.clone();
        let mut suffix = Vec::new();
        decoder.decode(&input[split..], &mut suffix);
        let mut cloned_suffix = Vec::new();
        for chunk in input[split..].chunks(1) {
            clone.decode(chunk, &mut cloned_suffix);
        }
        assert_eq!(suffix, cloned_suffix);
        prefix.extend_from_slice(&suffix);
        assert_eq!(prefix, expected);

        // Both retain the final "%X", even when output has been drained.
        suffix.clear();
        cloned_suffix.clear();
        decoder.decode(b"Y%41", &mut suffix);
        clone.decode(b"Y%41", &mut cloned_suffix);
        assert_eq!(suffix, b"%XYA");
        assert_eq!(cloned_suffix, suffix);
        decoder.reset();
        suffix.clear();
        decoder.decode(input, &mut suffix);
        assert_eq!(suffix, expected);
        decoder.reset();
        suffix.clear();
        decoder.decode(b"A", &mut suffix);
        assert_eq!(suffix, b"A");
    }
}

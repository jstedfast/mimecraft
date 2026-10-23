// Includes cases adapted from MimeKit's PassThroughEncoderTests and
// PassThroughDecoderTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentEncoding,
    encodings::{
        Base64Decoder, Base64Encoder, MimeDecoder, MimeEncoder, NewLineFormat, PassThroughDecoder,
        PassThroughEncoder, QuotedPrintableDecoder, QuotedPrintableEncoder, Rfc2231Decoder,
        UuDecoder, UuEncoder, YDecoder, YEncoder,
    },
};

#[test]
fn pass_through_preserves_every_byte_and_encoding_label() {
    let input: Vec<u8> = (0..=255).cycle().take(1024).collect();
    let encodings = [
        ContentEncoding::Unspecified,
        ContentEncoding::SevenBit,
        ContentEncoding::EightBit,
        ContentEncoding::Binary,
        ContentEncoding::Base64,
        ContentEncoding::QuotedPrintable,
        ContentEncoding::UuEncode,
    ];

    assert_eq!(ContentEncoding::default(), ContentEncoding::Unspecified);
    assert_eq!(
        PassThroughEncoder::default().encoding(),
        ContentEncoding::Unspecified
    );
    assert_eq!(
        PassThroughDecoder::default().encoding(),
        ContentEncoding::Unspecified
    );

    for encoding in encodings {
        for size in [1, 2, 3, 7, 64, 255, 1024, 1025] {
            let mut encoder = PassThroughEncoder::new(encoding);
            let mut decoder = PassThroughDecoder::new(encoding);
            let mut encoded = b"prefix".to_vec();
            let mut decoded = b"prefix".to_vec();
            for chunk in input.chunks(size) {
                encoder.encode(chunk, &mut encoded);
                encoder.encode(b"", &mut encoded);
                decoder.decode(chunk, &mut decoded);
                decoder.decode(b"", &mut decoded);
            }
            encoder.finish(&mut encoded);
            encoder.finish(&mut encoded);

            assert_eq!(&encoded[..6], b"prefix");
            assert_eq!(&encoded[6..], input);
            assert_eq!(decoded, encoded);
            assert_eq!(encoder.encoding(), encoding);
            assert_eq!(decoder.encoding(), encoding);
            assert_eq!(MimeEncoder::encoding(&encoder), encoding);
            assert_eq!(MimeDecoder::encoding(&decoder), encoding);

            let mut encoder = encoder.clone();
            let mut decoder = decoder.clone();
            MimeEncoder::reset(&mut encoder);
            MimeDecoder::reset(&mut decoder);
            assert_eq!(encoder.encoding(), encoding);
            assert_eq!(decoder.encoding(), encoding);

            // Clearing the output between chunks must not lose any input, and
            // finish must not introduce a final newline even after more data.
            encoded.clear();
            decoded.clear();
            for chunk in input.chunks(size) {
                MimeEncoder::encode(&mut encoder, chunk, &mut encoded);
                MimeDecoder::decode(&mut decoder, chunk, &mut decoded);
                assert_eq!(encoded, chunk);
                assert_eq!(decoded, chunk);
                encoded.clear();
                decoded.clear();
            }
            MimeEncoder::finish(&mut encoder, &mut encoded);
            assert!(encoded.is_empty());
        }
    }
}

fn assert_encoder<E: MimeEncoder + Clone>(template: E, encoding: ContentEncoding, expected: &[u8]) {
    let input = b"Cat";
    for split in 0..=input.len() {
        let mut encoder = template.clone();
        let mut output = b"prefix".to_vec();
        assert_eq!(encoder.encoding(), encoding);
        encoder.encode(&input[..split], &mut output);
        encoder.encode(b"", &mut output);

        let mut cloned = encoder.clone();
        let mut cloned_output = output.clone();
        let erased: &mut dyn MimeEncoder = &mut cloned;
        assert_eq!(erased.encoding(), encoding);
        for chunk in input[split..].chunks(1) {
            encoder.encode(chunk, &mut output);
            erased.encode(chunk, &mut cloned_output);
            erased.encode(b"", &mut cloned_output);
        }
        encoder.finish(&mut output);
        erased.finish(&mut cloned_output);
        assert_eq!(&output[..6], b"prefix");
        assert_eq!(&output[6..], expected, "split {split}");
        assert_eq!(cloned_output, output, "split {split}");

        // Reset both generic and erased encoders with partial input pending.
        encoder.reset();
        erased.reset();
        encoder.encode(b"discard", &mut output);
        erased.encode(b"discard", &mut cloned_output);
        encoder.reset();
        erased.reset();
        output.clear();
        cloned_output.clear();
        encoder.encode(input, &mut output);
        erased.encode(input, &mut cloned_output);
        encoder.finish(&mut output);
        erased.finish(&mut cloned_output);
        assert_eq!(output, expected);
        assert_eq!(cloned_output, expected);
        assert_eq!(encoder.encoding(), encoding);
        assert_eq!(erased.encoding(), encoding);
    }
}

#[test]
fn encoders_support_generic_and_dynamic_dispatch_with_clone_and_reset() {
    assert_encoder(Base64Encoder::new(), ContentEncoding::Base64, b"Q2F0\r\n");
    assert_encoder(
        QuotedPrintableEncoder::new(),
        ContentEncoding::QuotedPrintable,
        b"Cat=\r\n",
    );
    assert_encoder(
        UuEncoder::new(),
        ContentEncoding::UuEncode,
        b"#0V%T\r\n`\r\n",
    );
    assert_encoder(
        YEncoder::new(),
        ContentEncoding::Unspecified,
        b"m\x8b\x9e\r\n",
    );
    assert_encoder(
        PassThroughEncoder::new(ContentEncoding::Binary),
        ContentEncoding::Binary,
        b"Cat",
    );
}

#[test]
fn encoder_traits_preserve_configuration_across_clone_and_reset() {
    assert_encoder(
        Base64Encoder::with_options(60, NewLineFormat::Unix).unwrap(),
        ContentEncoding::Base64,
        b"Q2F0\n",
    );
    assert_encoder(
        QuotedPrintableEncoder::with_options(60, NewLineFormat::Unix).unwrap(),
        ContentEncoding::QuotedPrintable,
        b"Cat=\n",
    );
    assert_encoder(
        UuEncoder::with_new_line_format(NewLineFormat::Unix).unwrap(),
        ContentEncoding::UuEncode,
        b"#0V%T\n`\n",
    );
    assert_encoder(
        YEncoder::with_options(60, NewLineFormat::Unix).unwrap(),
        ContentEncoding::Unspecified,
        b"m\x8b\x9e\n",
    );
}

fn assert_decoder<D: MimeDecoder + Clone>(
    template: D,
    encoding: ContentEncoding,
    input: &[u8],
    expected: &[u8],
) {
    for split in 0..=input.len() {
        let mut decoder = template.clone();
        let mut output = b"prefix".to_vec();
        assert_eq!(decoder.encoding(), encoding);
        decoder.decode(&input[..split], &mut output);
        decoder.decode(b"", &mut output);

        let mut cloned = decoder.clone();
        let mut cloned_output = output.clone();
        let erased: &mut dyn MimeDecoder = &mut cloned;
        assert_eq!(erased.encoding(), encoding);
        for chunk in input[split..].chunks(1) {
            decoder.decode(chunk, &mut output);
            erased.decode(chunk, &mut cloned_output);
            erased.decode(b"", &mut cloned_output);
        }
        assert_eq!(&output[..6], b"prefix");
        assert_eq!(&output[6..], expected, "split {split}");
        assert_eq!(cloned_output, output, "split {split}");

        // Include partially received markers and escapes as well as streams
        // stopped by an end marker. Reset must retain the original decoder mode.
        decoder.reset();
        erased.reset();
        decoder.decode(&input[..split], &mut output);
        erased.decode(&input[..split], &mut cloned_output);
        decoder.reset();
        erased.reset();
        output.clear();
        cloned_output.clear();
        decoder.decode(input, &mut output);
        erased.decode(input, &mut cloned_output);
        assert_eq!(output, expected);
        assert_eq!(cloned_output, expected);
        assert_eq!(decoder.encoding(), encoding);
        assert_eq!(erased.encoding(), encoding);
    }
}

#[test]
fn decoders_support_generic_and_dynamic_dispatch_with_clone_and_reset() {
    assert_decoder(
        Base64Decoder::new(),
        ContentEncoding::Base64,
        b"Q2F0\r\n",
        b"Cat",
    );
    assert_decoder(
        QuotedPrintableDecoder::new(),
        ContentEncoding::QuotedPrintable,
        b"Cat=\r\n",
        b"Cat",
    );
    assert_decoder(
        UuDecoder::new(),
        ContentEncoding::UuEncode,
        b"begin 644 cat.txt\r\n#0V%T\r\n`\r\nend\r\n",
        b"Cat",
    );
    assert_decoder(
        UuDecoder::for_payload(),
        ContentEncoding::UuEncode,
        b"#0V%T\r\n`\r\n#0V%T\r\n",
        b"Cat",
    );
    assert_decoder(
        YDecoder::new(),
        ContentEncoding::Unspecified,
        b"=ybegin line=128 size=3 name=cat.txt\r\nm\x8b\x9e\r\n=yend size=3\r\n",
        b"Cat",
    );
    assert_decoder(
        YDecoder::for_payload(),
        ContentEncoding::Unspecified,
        b"m\x8b\x9e\r\n=yend\r\nignored",
        b"Cat",
    );
    assert_decoder(
        Rfc2231Decoder::new(),
        ContentEncoding::Unspecified,
        b"%43%61%74",
        b"Cat",
    );
    assert_decoder(
        PassThroughDecoder::new(ContentEncoding::EightBit),
        ContentEncoding::EightBit,
        b"\x00Cat\n\r\n\xff",
        b"\x00Cat\n\r\n\xff",
    );
}

#[test]
fn decoder_traits_preserve_tolerance_and_encoded_word_mode() {
    assert_decoder(
        Base64Decoder::new(),
        ContentEncoding::Base64,
        b"!Q2F0?Qw==Z",
        b"CatC",
    );
    assert_decoder(
        QuotedPrintableDecoder::new(),
        ContentEncoding::QuotedPrintable,
        b"x=~y=4Gz=4",
        b"x=~y=4Gz",
    );
    assert_decoder(
        QuotedPrintableDecoder::for_encoded_words(),
        ContentEncoding::QuotedPrintable,
        b"Cat_word=3F",
        b"Cat word?",
    );
    assert_decoder(
        Rfc2231Decoder::new(),
        ContentEncoding::Unspecified,
        b"x%~y%4Gz%4",
        b"x%~y%4Gz",
    );
}

#[test]
fn trait_finish_preserves_codec_specific_behavior() {
    let mut base64 = Base64Encoder::new();
    let mut output = Vec::new();
    MimeEncoder::encode(&mut base64, b"C", &mut output);
    assert!(output.is_empty());
    MimeEncoder::finish(&mut base64, &mut output);
    MimeEncoder::finish(&mut base64, &mut output);
    assert_eq!(output, b"Qw==\r\n");

    let mut quoted_printable = QuotedPrintableEncoder::new();
    output.clear();
    MimeEncoder::encode(&mut quoted_printable, b"Cat ", &mut output);
    MimeEncoder::finish(&mut quoted_printable, &mut output);
    MimeEncoder::finish(&mut quoted_printable, &mut output);
    assert_eq!(output, b"Cat=20=\r\n");

    let mut uu = UuEncoder::new();
    output.clear();
    MimeEncoder::finish(&mut uu, &mut output);
    MimeEncoder::finish(&mut uu, &mut output);
    assert_eq!(output, b"`\r\n`\r\n");

    let mut yenc = YEncoder::new();
    output.clear();
    MimeEncoder::encode(&mut yenc, b"123456789", &mut output);
    let checksum = yenc.checksum();
    assert_eq!(!checksum, 0xcbf43926);
    MimeEncoder::finish(&mut yenc, &mut output);
    let finished = output.clone();
    MimeEncoder::finish(&mut yenc, &mut output);
    assert_eq!(output, finished);
    assert_eq!(yenc.checksum(), checksum);
    MimeEncoder::reset(&mut yenc);
    assert_eq!(yenc.checksum(), u32::MAX);

    let mut decoder = YDecoder::for_payload();
    let mut decoded = Vec::new();
    MimeDecoder::decode(&mut decoder, &output, &mut decoded);
    assert_eq!(decoded, b"123456789");
    assert_eq!(decoder.checksum(), checksum);
    MimeDecoder::reset(&mut decoder);
    assert_eq!(decoder.checksum(), u32::MAX);
}

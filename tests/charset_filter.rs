// Includes cases adapted from MimeKit's UnitTests/IO/Filters/FilterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Write};

use mimecraft::{
    charsets::{Charset, CharsetError},
    encodings::Base64Encoder,
    filters::{CharsetFilter, EncoderFilter, MimeFilter},
    io::{FilteredReader, FilteredWriter, MeasuringStream},
};

fn check_splits(
    source: Charset,
    target: Charset,
    input: &[u8],
    expected: &[u8],
    decode_errors: bool,
    encode_errors: bool,
) {
    for split in 0..=input.len() {
        let mut filter = CharsetFilter::new(source, target);
        let mut output = b"prefix:".to_vec();
        filter.filter(&input[..split], &mut output);
        filter.filter(b"", &mut output);
        filter.filter(&input[split..], &mut output);
        filter.finish(&mut output);
        filter.finish(&mut output);
        assert_eq!(&output[..7], b"prefix:");
        assert_eq!(
            &output[7..],
            expected,
            "{source} -> {target}, split {split}"
        );
        assert_eq!(filter.had_decode_errors(), decode_errors, "split {split}");
        assert_eq!(filter.had_encode_errors(), encode_errors, "split {split}");
        assert_eq!(filter.had_errors(), decode_errors || encode_errors);
    }
}

#[test]
fn constructor_reuses_charset_aliases_and_rejects_unsupported_labels() {
    let filter = CharsetFilter::from_labels("CP65001", "ISO_8859_1").unwrap();
    assert_eq!(filter.source_charset(), Charset::UTF8);
    assert_eq!(filter.target_charset(), Charset::LATIN1);
    assert!(!filter.had_errors());
    for label in [
        "bogus charset",
        "utf-7",
        "utf-32",
        "replacement",
        "x-user-defined",
    ] {
        for (source, target) in [(label, "utf-8"), ("utf-8", label)] {
            assert!(matches!(
                CharsetFilter::from_labels(source, target),
                Err(CharsetError::UnsupportedLabel(name)) if name == label
            ));
        }
    }
}

#[test]
fn canonical_french_conversion_reports_explicit_target_fallback() {
    let french = "Wikip\u{e9}dia est un projet d\u{2019}encyclop\u{e9}die.\n\
                  Tous les r\u{e9}dacteurs sont b\u{e9}n\u{e9}voles.";
    // Try converting with fallback: the curly apostrophe does not fit Latin-1.
    let expected = french.replace('\u{2019}', "?");
    check_splits(
        Charset::UTF8,
        Charset::LATIN1,
        french.as_bytes(),
        &Charset::LATIN1.encode(&expected).unwrap(),
        false,
        true,
    );
    // Try converting with no fallback.
    let latin9 = Charset::from_label("iso-8859-15").unwrap();
    let text = "Wikip\u{e9}dia: 10 \u{20ac}.";
    check_splits(
        Charset::UTF8,
        latin9,
        text.as_bytes(),
        &latin9.encode(text).unwrap(),
        false,
        false,
    );
}

#[test]
fn unicode_conversions_preserve_boms_nul_and_split_surrogate_pairs() {
    let text = "\u{feff}A\0Ren\u{e9}e \u{65e5}\u{672c}\u{8a9e} \u{1f600}\r\n";
    let charsets = [
        Charset::UTF8,
        Charset::from_label("utf-16").unwrap(),
        Charset::from_label("utf-16be").unwrap(),
    ];
    for source in charsets {
        for target in charsets {
            check_splits(
                source,
                target,
                &source.encode(text).unwrap(),
                &target.encode(text).unwrap(),
                false,
                false,
            );
        }
    }
}

#[test]
fn multibyte_and_stateful_charsets_survive_every_input_split() {
    for (label, text) in [
        ("shift_jis", "A\u{65e5}\u{672c}\u{8a9e}Z"),
        ("euc-jp", "\u{65e5}\u{672c}\u{8a9e}"),
        ("iso-2022-jp", "A\u{65e5}\u{672c}B\u{8a9e}"),
        ("euc-kr", "A\u{d55c}\u{ad6d}\u{c5b4}Z"),
        ("big5", "A\u{4e2d}\u{6587}Z"),
        ("gbk", "\u{4e2d}\u{6587}"),
        ("gb18030", "\u{4e2d}\u{6587}\u{1f600}"),
        ("windows-1251", "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}"),
        ("koi8-r", "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}"),
    ] {
        let charset = Charset::from_label(label).unwrap();
        let encoded = charset.encode(text).unwrap();
        check_splits(
            charset,
            Charset::UTF8,
            &encoded,
            text.as_bytes(),
            false,
            false,
        );
        check_splits(
            Charset::UTF8,
            charset,
            text.as_bytes(),
            &encoded,
            false,
            false,
        );
    }
}

#[test]
fn single_byte_decoding_matches_complete_buffer_policy_for_every_byte() {
    let input: Vec<u8> = (0..=255).collect();
    for label in [
        "us-ascii",
        "iso-8859-1",
        "windows-1252",
        "windows-1251",
        "iso-8859-2",
        "iso-8859-15",
        "koi8-r",
        "ibm866",
        "macintosh",
    ] {
        let charset = Charset::from_label(label).unwrap();
        let (expected, errors) = charset.decode_lossy(&input);
        check_splits(
            charset,
            Charset::UTF8,
            &input,
            expected.as_bytes(),
            errors,
            false,
        );
    }
}

#[test]
fn ascii_and_latin1_are_not_browser_windows1252_aliases() {
    check_splits(
        Charset::LATIN1,
        Charset::UTF8,
        b"\x80\x9f\xff",
        "\u{80}\u{9f}\u{ff}".as_bytes(),
        false,
        false,
    );
    check_splits(
        Charset::ASCII,
        Charset::UTF8,
        b"\x80\x9f\xff",
        "\u{fffd}\u{fffd}\u{fffd}".as_bytes(),
        true,
        false,
    );
    check_splits(Charset::ASCII, Charset::LATIN1, b"\xffA", b"?A", true, true);
    check_splits(
        Charset::LATIN1,
        Charset::ASCII,
        b"\xffA",
        b"?A",
        false,
        true,
    );
}

#[test]
fn malformed_sequences_match_complete_decoding_regardless_of_chunks() {
    let cases: &[(&str, &[u8])] = &[
        ("utf-8", b"\xff\xc0\xafA\xe2(\xa1B\xf0\x9f"),
        ("utf-16", b"\x00\xd8A\x00\x00\xdcZ"),
        ("utf-16be", b"\xd8\x00\x00A\xdc\x00Z"),
        ("shift_jis", b"\x82 \x82"),
        ("euc-jp", b"\x8f\xa1"),
        ("gb18030", b"\x81\x30\x81"),
        ("iso-2022-jp", b"A\x1b$B$"),
    ];
    for &(label, input) in cases {
        let source = Charset::from_label(label).unwrap();
        let (expected, errors) = source.decode_lossy(input);
        assert!(errors, "{label}");
        check_splits(
            source,
            Charset::UTF8,
            input,
            expected.as_bytes(),
            true,
            false,
        );
    }
}

#[test]
fn incomplete_input_is_diagnosed_only_when_finished() {
    let mut filter = CharsetFilter::new(Charset::UTF8, Charset::UTF8);
    let mut output = Vec::new();
    filter.filter(b"\xf0\x9f", &mut output);
    assert!(output.is_empty());
    assert!(!filter.had_errors());
    filter.filter(b"", &mut output);
    assert!(!filter.had_errors());
    filter.finish(&mut output);
    assert_eq!(output, "\u{fffd}".as_bytes());
    assert!(filter.had_decode_errors());
    assert!(!filter.had_encode_errors());
    filter.finish(&mut output);
    filter.filter(b"", &mut output);
    assert_eq!(output, "\u{fffd}".as_bytes());
}

#[test]
fn source_replacement_and_target_replacement_are_reported_independently() {
    for (input, decoded, encoded, expected) in [
        (b"A".as_slice(), false, false, b"A".as_slice()),
        ("\u{fffd}".as_bytes(), false, true, b"?"),
        (b"\xff", true, true, b"?"),
        ("\u{1f600}\u{1f600}".as_bytes(), false, true, b"??"),
    ] {
        check_splits(
            Charset::UTF8,
            Charset::ASCII,
            input,
            expected,
            decoded,
            encoded,
        );
    }
}

#[test]
fn legacy_target_fallback_is_question_marks_not_html_references_or_best_fit() {
    let text = "A\u{1f600}\u{2019}\u{65e5}Z";
    for (label, expected) in [
        ("us-ascii", b"A???Z".as_slice()),
        ("iso-8859-1", b"A???Z"),
        ("iso-8859-15", b"A???Z"),
        ("windows-1252", b"A?\x92?Z"),
    ] {
        check_splits(
            Charset::UTF8,
            Charset::from_label(label).unwrap(),
            text.as_bytes(),
            expected,
            false,
            true,
        );
    }
}

#[test]
fn stateful_target_fallback_preserves_shift_sequences_and_following_characters() {
    let target = Charset::from_label("iso-2022-jp").unwrap();
    let text = "\u{65e5}\u{1f600}\u{672c}\u{1f600}";
    let expected = target.encode("\u{65e5}?\u{672c}?").unwrap();
    check_splits(
        Charset::UTF8,
        target,
        text.as_bytes(),
        &expected,
        false,
        true,
    );

    let mut filter = CharsetFilter::new(Charset::UTF8, target);
    let mut output = Vec::new();
    filter.filter("\u{65e5}".as_bytes(), &mut output);
    assert!(!output.ends_with(b"\x1b(B"));
    let before = output.len();
    filter.finish(&mut output);
    assert_eq!(output.len(), before + 3);
    assert!(output.ends_with(b"\x1b(B"));
    filter.finish(&mut output);
    assert_eq!(output.len(), before + 3);
}

#[test]
fn buffer_exhaustion_does_not_lose_decoded_characters_or_encoder_state() {
    for (source_label, target_label, text) in [
        ("utf-8", "utf-16be", "x\u{1f600}\u{e9}"),
        ("utf-16", "utf-8", "\u{1f600}x\u{e9}"),
        ("iso-2022-jp", "utf-16", "A\u{65e5}\u{672c}"),
        ("utf-8", "iso-2022-jp", "A\u{65e5}\u{672c}"),
        ("iso-8859-1", "utf-8", "A\u{ff}\u{80}"),
    ] {
        let text = text.repeat(5000);
        let source = Charset::from_label(source_label).unwrap();
        let target = Charset::from_label(target_label).unwrap();
        let input = source.encode(&text).unwrap();
        let expected = target.encode(&text).unwrap();
        for size in [1, 17, 4093, input.len()] {
            let mut filter = CharsetFilter::new(source, target);
            let mut output = Vec::new();
            for chunk in input.chunks(size) {
                filter.filter(chunk, &mut output);
            }
            filter.finish(&mut output);
            assert_eq!(
                output,
                expected.as_ref(),
                "{source} -> {target}, chunk {size}"
            );
            assert!(!filter.had_errors());
        }
    }
}

#[test]
fn reset_discards_partial_input_shift_state_and_sticky_diagnostics() {
    let mut filter = CharsetFilter::new(Charset::UTF8, Charset::ASCII);
    let mut output = Vec::new();
    filter.filter(b"\xff\xf0\x9f", &mut output);
    assert!(filter.had_decode_errors());
    assert!(filter.had_encode_errors());
    filter.reset();
    output.clear();
    assert!(!filter.had_errors());
    assert_eq!(filter.source_charset(), Charset::UTF8);
    assert_eq!(filter.target_charset(), Charset::ASCII);
    filter.filter(b"fresh", &mut output);
    filter.finish(&mut output);
    assert_eq!(output, b"fresh");
    assert!(!filter.had_errors());
    filter.reset();
    output.clear();
    filter.finish(&mut output);
    assert!(output.is_empty());

    let jp = Charset::from_label("iso-2022-jp").unwrap();
    let mut filter = CharsetFilter::new(Charset::UTF8, jp);
    filter.filter("\u{65e5}".as_bytes(), &mut output);
    filter.reset();
    output.clear();
    filter.filter(b"ASCII", &mut output);
    filter.finish(&mut output);
    assert_eq!(output, b"ASCII");

    let mut filter = CharsetFilter::new(jp, Charset::UTF8);
    filter.filter(b"\x1b$", &mut output);
    filter.reset();
    output.clear();
    filter.filter(b"ASCII", &mut output);
    filter.finish(&mut output);
    assert_eq!(output, b"ASCII");
    assert!(!filter.had_errors());
}

#[test]
fn arbitrary_bytes_match_complete_decoding_across_internal_buffer_boundaries() {
    let mut seed = 0xc4b9_3ef1_u32;
    let input: Vec<u8> = (0..16385)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as u8
        })
        .collect();
    for label in [
        "us-ascii",
        "iso-8859-1",
        "utf-8",
        "utf-16",
        "utf-16be",
        "shift_jis",
        "euc-jp",
        "iso-2022-jp",
        "euc-kr",
        "big5",
        "gb18030",
    ] {
        let source = Charset::from_label(label).unwrap();
        let (expected, errors) = source.decode_lossy(&input);
        for size in [1, 37, 4096, input.len()] {
            let mut filter = CharsetFilter::new(source, Charset::UTF8);
            let mut output = Vec::new();
            for chunk in input.chunks(size) {
                filter.filter(chunk, &mut output);
            }
            filter.finish(&mut output);
            assert_eq!(output, expected.as_bytes(), "{label}, chunk {size}");
            assert_eq!(filter.had_decode_errors(), errors);
            assert!(!filter.had_encode_errors());
        }
    }
}

#[test]
fn repeated_target_fallback_preserves_state_through_internal_buffer_exhaustion() {
    let text = "\u{65e5}\u{1f600}\u{672c}X".repeat(3000);
    let target = Charset::from_label("iso-2022-jp").unwrap();
    let expected = target
        .encode(&text.replace('\u{1f600}', "?"))
        .unwrap()
        .into_owned();
    for size in [1, 29, 4096, text.len()] {
        let mut filter = CharsetFilter::new(Charset::UTF8, target);
        let mut output = Vec::new();
        for chunk in text.as_bytes().chunks(size) {
            filter.filter(chunk, &mut output);
        }
        filter.finish(&mut output);
        assert_eq!(output, expected);
        assert!(filter.had_encode_errors());
        assert!(!filter.had_decode_errors());
    }
}

#[test]
#[should_panic(expected = "reset CharsetFilter before reusing it")]
fn nonempty_input_after_finish_requires_reset() {
    let mut filter = CharsetFilter::new(Charset::UTF8, Charset::UTF8);
    let mut output = Vec::new();
    filter.finish(&mut output);
    filter.filter(b"new stream", &mut output);
}

#[test]
fn callers_can_drain_output_and_use_boxed_trait_objects() {
    let mut filter: Box<dyn MimeFilter> =
        Box::new(CharsetFilter::new(Charset::UTF8, Charset::LATIN1));
    let mut buffer = Vec::new();
    let mut output = Vec::new();
    for &byte in "Ren\u{e9}e".as_bytes() {
        filter.filter(&[byte], &mut buffer);
        output.append(&mut buffer);
    }
    filter.finish(&mut buffer);
    output.append(&mut buffer);
    assert_eq!(output, b"Ren\xe9e");
    filter.reset();
    filter.filter(b"again", &mut buffer);
    filter.finish(&mut buffer);
    assert_eq!(buffer, b"again");
}

#[test]
fn filtered_readers_finalize_incomplete_source_bytes_at_eof() {
    let mut reader = FilteredReader::new(
        Cursor::new(b"Ren\xc3\xa9e\xf0\x9f"),
        CharsetFilter::new(Charset::UTF8, Charset::UTF8),
    );
    let mut output = Vec::new();
    let mut byte = [0; 1];
    while reader.read(&mut byte).unwrap() != 0 {
        output.extend_from_slice(&byte);
    }
    assert_eq!(output, "Ren\u{e9}e\u{fffd}".as_bytes());
    assert!(reader.filter().had_decode_errors());
}

#[test]
fn nested_writers_convert_charset_before_transfer_encoding_and_measurement() {
    let inner = FilteredWriter::new(Vec::new(), EncoderFilter::new(Base64Encoder::new()));
    let mut outer = FilteredWriter::new(inner, CharsetFilter::new(Charset::UTF8, Charset::LATIN1));
    outer.write_all(b"\xc3").unwrap();
    outer.flush().unwrap();
    assert!(outer.get_ref().get_ref().is_empty());
    outer.write_all(b"\xa9").unwrap();
    outer.finish().unwrap();
    outer.get_mut().finish().unwrap();
    assert_eq!(outer.get_ref().get_ref(), b"6Q==\r\n");
    assert!(!outer.filter().had_errors());

    let mut writer = FilteredWriter::new(
        MeasuringStream::new(),
        CharsetFilter::from_labels("utf-8", "utf-16be").unwrap(),
    );
    writer.write_all("A\u{1f600}".as_bytes()).unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().len(), 6);
}

#[derive(Default)]
struct RetrySink {
    bytes: Vec<u8>,
    fail: bool,
}

impl Write for RetrySink {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.fail) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "try again"));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn retrying_failed_final_output_does_not_finalize_charset_twice() {
    let sink = RetrySink {
        fail: true,
        ..RetrySink::default()
    };
    let mut writer = FilteredWriter::new(sink, CharsetFilter::new(Charset::UTF8, Charset::UTF8));
    writer.write_all(b"\xf0\x9f").unwrap();
    assert_eq!(
        writer.finish().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(writer.filter().had_decode_errors());
    writer.finish().unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().bytes, "\u{fffd}".as_bytes());
}

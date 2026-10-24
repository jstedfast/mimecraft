// Adapted from MimeKit's Text/PlainTextPreviewer.cs and its unit tests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Cursor, Read};

use mimecraft::{
    MimeMessage, TextFormat, TextPart,
    text::{PlainTextPreviewer, TextToFlowed},
};

fn preview(limit: usize, text: &str) -> String {
    let previewer = PlainTextPreviewer::with_maximum_preview_length(limit).unwrap();
    let expected = previewer.preview(text);
    let mut reader = text.as_bytes();
    assert_eq!(previewer.preview_reader(&mut reader).unwrap(), expected);
    expected
}

#[test]
fn defaults_and_length_bounds() {
    let previewer = PlainTextPreviewer::new();
    assert_eq!(previewer, PlainTextPreviewer::default());
    assert_eq!(previewer.maximum_preview_length(), 230);
    for limit in [1, 230, 1024] {
        let previewer = PlainTextPreviewer::with_maximum_preview_length(limit).unwrap();
        assert_eq!(previewer.maximum_preview_length(), limit);
    }
    for limit in [0, 1025, usize::MAX] {
        let error = PlainTextPreviewer::with_maximum_preview_length(limit).unwrap_err();
        assert_eq!(error.requested(), limit);
        assert_eq!(
            error.to_string(),
            format!("preview length must be between 1 and 1024 characters, got {limit}")
        );
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn empty_and_whitespace_only_input() {
    for limit in [1, 2, 230, 1024] {
        for text in [
            "",
            " ",
            "\t\r\n\u{b}\u{c}",
            "\u{a0}\u{200b}\u{200c}\u{200d}",
        ] {
            assert_eq!(preview(limit, text), "");
        }
    }
}

#[test]
fn collapses_ascii_whitespace_without_a_final_newline() {
    assert_eq!(
        preview(230, "\t First \r\n\r\nsecond \t third\rfourth\n "),
        "First second third fourth"
    );
    assert_eq!(preview(230, "word"), "word");
    assert_eq!(preview(230, "word\r\n"), "word");
}

#[test]
fn recognizes_unicode_whitespace_and_source_zero_width_characters() {
    let whitespace = [
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}',
        '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
        '\u{2008}', '\u{2009}', '\u{200a}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{2028}',
        '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ];
    for character in whitespace {
        let text = format!("{character}a{character}{character}b{character}");
        assert_eq!(preview(230, &text), "a b", "{character:?}");
    }
    for character in [
        '\0', '\u{1c}', '\u{180e}', '\u{200e}', '\u{2060}', '\u{feff}',
    ] {
        let text = format!("{character}a{character}");
        assert_eq!(preview(230, &text), text, "{character:?}");
    }
}

#[test]
fn truncation_matches_the_source_string_overload() {
    for (limit, input, expected) in [
        (1, "a", "a"),
        (1, "ab", "\u{2026}"),
        (1, "a ", "\u{2026}"),
        (1, "  a", "a"),
        (2, "abc", "a\u{2026}"),
        (2, "a b", "\u{2026}"),
        (3, "abc", "abc"),
        (3, "abcd", "ab\u{2026}"),
        (3, "abc ", "ab\u{2026}"),
        (3, "ab ", "ab"),
        (3, "ab  ", "a\u{2026}"),
        (3, "ab cd", "a\u{2026}"),
        (3, "a bc", "a \u{2026}"),
        (3, "a  b", "a b"),
        (4, "abc def", "ab\u{2026}"),
    ] {
        assert_eq!(preview(limit, input), expected, "{limit}: {input:?}");
    }
}

#[test]
fn default_and_maximum_limits_include_the_ellipsis() {
    for limit in [230, 1024] {
        let exact = "a".repeat(limit);
        assert_eq!(preview(limit, &exact), exact);
        let expected = format!("{}\u{2026}", "a".repeat(limit - 1));
        assert_eq!(preview(limit, &(exact + "b")), expected);
        assert_eq!(expected.chars().count(), limit);
    }
}

#[test]
fn limits_count_unicode_scalars_not_bytes_or_utf16_units() {
    let text = "\u{1f408}\u{65e5}\u{e9}";
    assert_eq!(preview(3, text), text);
    assert_eq!(preview(2, text), "\u{1f408}\u{2026}");
    assert_eq!(preview(1, text), "\u{2026}");
    assert_eq!(preview(4, "\u{1f408}ab\u{1f408}z"), "\u{1f408}ab\u{2026}");
    assert_eq!(preview(2, "e\u{301}x"), "e\u{2026}");
    assert_eq!(
        preview(230, "\u{1f469}\u{200d}\u{1f4bb}"),
        "\u{1f469} \u{1f4bb}"
    );
}

#[test]
fn plain_text_is_not_interpreted_as_html_or_flowed() {
    assert_eq!(preview(230, "<b>Hello</b> &amp;"), "<b>Hello</b> &amp;");
    assert_eq!(preview(230, "> one \n> two\n-- \n"), "> one > two --");
}

#[test]
fn repeated_calls_are_independent_and_large_input_is_bounded() {
    let previewer = PlainTextPreviewer::with_maximum_preview_length(4).unwrap();
    let text = format!("{}first{}", " ".repeat(100_000), "x".repeat(1_000_000));
    assert_eq!(previewer.preview(&text), "fir\u{2026}");
    assert_eq!(previewer.preview("next"), "next");
    assert_eq!(previewer.preview(""), "");
    assert_eq!(previewer.preview(&" ".repeat(100_000)), "");
    let copy = previewer;
    assert_eq!(copy.preview("other"), "oth\u{2026}");
}

#[test]
fn varied_unicode_and_spacing_always_respect_scalar_limit() {
    let tokens = ["", " ", "\r\n", "a", "\u{1f408}", "\u{301}", "\u{200b}"];
    for limit in 1..=8 {
        for first in tokens {
            for second in tokens {
                for third in tokens {
                    let text = format!("{first}{second}{third}end");
                    let result = preview(limit, &text);
                    assert!(result.chars().count() <= limit, "{limit}: {text:?}");
                    assert!(!result.starts_with(' '));
                    assert!(!result.ends_with(' '));
                    assert!(!result.contains("  "));
                }
            }
        }
    }
}

#[test]
fn previews_compose_with_decoded_flowed_message_bodies() {
    let original = format!("From someone\r\n> {}end\r\n", "words ".repeat(40));
    let flowed = TextToFlowed::new().convert(&original);
    let mut part = TextPart::new("plain", &flowed).unwrap();
    let mut content_type = part.part().content_type().unwrap().unwrap();
    content_type
        .parameters_mut()
        .set("format", "flowed")
        .unwrap();
    content_type.parameters_mut().set("delsp", "yes").unwrap();
    part.part_mut().set_content_type(&content_type).unwrap();
    let mut message = MimeMessage::with_body(part.into());
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let mut loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    let text = loaded.text_body(TextFormat::Plain).unwrap().unwrap();
    assert_eq!(text, original);
    assert_eq!(preview(24, &text), "From someone > words wo\u{2026}");
}

#[test]
fn streaming_matches_strings_across_utf8_and_buffer_boundaries() {
    let samples = [
        "",
        "a",
        "  abc \r\n next  ",
        "\u{feff}abc",
        "\0a\0",
        "\u{e9}\u{65e5}\u{1f408}e\u{301}",
        "\u{200b}a\u{200c}\u{200d}b\u{85}\u{a0}c\u{3000}",
        "\u{1f469}\u{200d}\u{1f4bb}",
    ];
    for capacity in 1..=16 {
        for limit in 1..=12 {
            let previewer = PlainTextPreviewer::with_maximum_preview_length(limit).unwrap();
            for text in samples {
                let mut reader = BufReader::with_capacity(capacity, text.as_bytes());
                assert_eq!(
                    previewer.preview_reader(&mut reader).unwrap(),
                    previewer.preview(text),
                    "capacity {capacity}, limit {limit}, input {text:?}"
                );
            }
        }
    }
    for limit in [230, 1024] {
        let previewer = PlainTextPreviewer::with_maximum_preview_length(limit).unwrap();
        for extra in [0, 1] {
            let text = "\u{1f408}".repeat(limit + extra);
            let mut reader = BufReader::with_capacity(3, text.as_bytes());
            let result = previewer.preview_reader(&mut reader).unwrap();
            assert_eq!(result, previewer.preview(&text));
            assert_eq!(result.chars().count(), limit);
        }
    }
}

#[test]
fn streaming_leaves_the_suffix_in_the_callers_reader() {
    let previewer = PlainTextPreviewer::with_maximum_preview_length(3).unwrap();
    let input = "ignored\u{e9}\u{65e5}\u{1f408} suffix";
    let mut cursor = Cursor::new(input.as_bytes());
    cursor.set_position(7);
    assert_eq!(
        previewer.preview_reader(&mut cursor).unwrap(),
        "\u{e9}\u{65e5}\u{2026}"
    );
    assert_eq!(cursor.position(), 7 + 2 + 3 + 4);
    let mut reader = BufReader::with_capacity(64, cursor);
    assert_eq!(previewer.preview_reader(&mut reader).unwrap(), "su\u{2026}");
    let mut suffix = String::new();
    reader.read_to_string(&mut suffix).unwrap();
    assert_eq!(suffix, "fix");

    let mut input = b"ab cd".as_slice();
    let reader: &mut dyn BufRead = &mut input;
    assert_eq!(previewer.preview_reader(reader).unwrap(), "a\u{2026}");
    assert_eq!(input, b"cd");
}

#[test]
fn streaming_rejects_only_malformed_utf8_that_it_consumes() {
    let malformed: &[&[u8]] = &[
        b"\x80",
        b"\xff",
        b"\xc0\xaf",
        b"\xc2",
        b"\xc2x",
        b"\xe0\x80\x80",
        b"\xed\xa0\x80",
        b"\xe2",
        b"\xe2\x82",
        b"\xe2x\xac",
        b"\xf0\x80\x80\x80",
        b"\xf4\x90\x80\x80",
        b"\xf5\x80\x80\x80",
        b"\xf0",
        b"\xf0\x9f",
        b"\xf0\x9f\x90",
    ];
    for bytes in malformed {
        for capacity in 1..=5 {
            let mut input = b"ok".to_vec();
            input.extend_from_slice(bytes);
            let mut reader = BufReader::with_capacity(capacity, input.as_slice());
            let error = PlainTextPreviewer::new()
                .preview_reader(&mut reader)
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(
                error
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<std::str::Utf8Error>()
                    .is_some()
            );
        }
        let previewer = PlainTextPreviewer::with_maximum_preview_length(1).unwrap();
        let mut input = vec![b'a'];
        input.extend_from_slice(bytes);
        let mut reader = input.as_slice();
        assert_eq!(previewer.preview_reader(&mut reader).unwrap(), "\u{2026}");
        assert_eq!(reader, *bytes);
        assert!(previewer.preview_reader(&mut reader).is_err());
    }
}

struct FaultReader<'a> {
    bytes: &'a [u8],
    position: usize,
    fail_at: Option<usize>,
    kind: io::ErrorKind,
    interrupt_next: bool,
}

impl<'a> FaultReader<'a> {
    fn new(bytes: &'a [u8], fail_at: Option<usize>, kind: io::ErrorKind) -> Self {
        Self {
            bytes,
            position: 0,
            fail_at,
            kind,
            interrupt_next: true,
        }
    }
}

impl Read for FaultReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for FaultReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.interrupt_next = !self.interrupt_next;
        if !self.interrupt_next {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_at == Some(self.position) {
            return Err(io::Error::new(self.kind, "preview source failure"));
        }
        Ok(&self.bytes[self.position..(self.position + 1).min(self.bytes.len())])
    }

    fn consume(&mut self, amount: usize) {
        assert!(amount <= 1);
        self.position += amount;
        assert!(self.position <= self.bytes.len());
    }
}

#[test]
fn streaming_retries_interruptions_and_propagates_source_errors_without_partial_output() {
    let text = " \u{e9}\u{65e5}\u{1f408}\r\nend";
    let previewer = PlainTextPreviewer::new();
    for kind in [
        io::ErrorKind::Other,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::UnexpectedEof,
    ] {
        for fail_at in 0..=text.len() {
            let mut reader = FaultReader::new(text.as_bytes(), Some(fail_at), kind);
            let error = previewer.preview_reader(&mut reader).unwrap_err();
            assert_eq!(error.kind(), kind, "source failure at {fail_at}");
            assert_eq!(error.to_string(), "preview source failure");
            assert_eq!(reader.position, fail_at);
        }
    }
    let mut reader = FaultReader::new(text.as_bytes(), None, io::ErrorKind::Other);
    assert_eq!(
        previewer.preview_reader(&mut reader).unwrap(),
        previewer.preview(text)
    );
    assert_eq!(reader.position, text.len());
    assert_eq!(previewer.preview_reader(&mut reader).unwrap(), "");
}

#[test]
fn streaming_checks_eof_at_the_limit_without_draining_or_hiding_lookahead_failures() {
    let previewer = PlainTextPreviewer::with_maximum_preview_length(3).unwrap();
    for input in [b"abc".as_slice(), b"abc suffix"] {
        let mut reader = FaultReader::new(input, Some(3), io::ErrorKind::PermissionDenied);
        let error = previewer.preview_reader(&mut reader).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(reader.position, 3);
    }
    let mut reader = FaultReader::new(b"abc suffix", Some(4), io::ErrorKind::Other);
    assert_eq!(previewer.preview_reader(&mut reader).unwrap(), "ab\u{2026}");
    assert_eq!(reader.position, 3);
    let mut reader = FaultReader::new(b"abc", None, io::ErrorKind::Other);
    assert_eq!(previewer.preview_reader(&mut reader).unwrap(), "abc");
    assert_eq!(reader.position, 3);

    // A never-ending source proves the previewer does not collect or drain it.
    let mut reader = BufReader::with_capacity(7, io::repeat(b'x'));
    assert_eq!(previewer.preview_reader(&mut reader).unwrap(), "xx\u{2026}");
    assert_eq!(reader.buffer(), b"xxxx");
}

#[test]
fn streaming_scans_long_whitespace_runs_with_bounded_preview_storage() {
    let previewer = PlainTextPreviewer::with_maximum_preview_length(4).unwrap();
    let input = io::repeat(b' ').take(100_000).chain(b"one two".as_slice());
    let mut reader = BufReader::with_capacity(13, input);
    assert_eq!(previewer.preview_reader(&mut reader).unwrap(), "on\u{2026}");
    let mut suffix = String::new();
    reader.read_to_string(&mut suffix).unwrap();
    assert_eq!(suffix, "two");
    let mut reader = BufReader::with_capacity(13, io::repeat(b' ').take(100_000));
    assert_eq!(previewer.preview_reader(&mut reader).unwrap(), "");
}

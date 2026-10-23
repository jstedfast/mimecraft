// Includes synchronous cases adapted from MimeKit's UnitTests/MimeContentTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::VecDeque,
    io::{self, BufRead, Cursor, ErrorKind, Read, Seek, SeekFrom, Write},
};

use mimecraft::{
    ContentEncoding, MimeContent, encodings::NewLineFormat, filters::encoder_filter,
    io::FilteredWriter,
};

enum Step {
    Limit(usize),
    Error(ErrorKind),
}

struct Source {
    cursor: Cursor<Vec<u8>>,
    read_limit: usize,
    reads: usize,
    steps: VecDeque<Step>,
    seeks: Vec<SeekFrom>,
    fail_seek: Option<usize>,
}

impl Source {
    fn new(input: &[u8], read_limit: usize) -> Self {
        Self {
            cursor: Cursor::new(input.to_vec()),
            read_limit,
            reads: 0,
            steps: VecDeque::new(),
            seeks: Vec::new(),
            fail_seek: None,
        }
    }
}

impl Read for Source {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        let limit = match self.steps.pop_front() {
            Some(Step::Limit(limit)) => limit,
            Some(Step::Error(kind)) => return Err(io::Error::new(kind, "source read failure")),
            None => self.read_limit,
        };
        let count = output.len().min(limit);
        self.cursor.read(&mut output[..count])
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks.push(position);
        if self.fail_seek == Some(self.seeks.len()) {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "source rewind failure",
            ));
        }
        self.cursor.seek(position)
    }
}

struct Sink {
    bytes: Vec<u8>,
    write_limit: usize,
    steps: VecDeque<Step>,
    flushes: usize,
    flush_error: Option<ErrorKind>,
}

impl Sink {
    fn new(write_limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            write_limit,
            steps: VecDeque::new(),
            flushes: 0,
            flush_error: None,
        }
    }
}

impl Write for Sink {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let limit = match self.steps.pop_front() {
            Some(Step::Limit(limit)) => limit,
            Some(Step::Error(kind)) => return Err(io::Error::new(kind, "sink write failure")),
            None => self.write_limit,
        };
        let count = input.len().min(limit);
        self.bytes.extend_from_slice(&input[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        match self.flush_error.take() {
            Some(kind) => Err(io::Error::new(kind, "sink flush failure")),
            None => Ok(()),
        }
    }
}

// Also exercise the public methods with an unsized destination trait object.
fn copy(content: &mut MimeContent<Source>, sink: &mut dyn Write, decode: bool) -> io::Result<u64> {
    if decode {
        content.decode_to(sink)
    } else {
        content.write_to(sink)
    }
}

const CASES: &[(ContentEncoding, &[u8], &[u8])] = &[
    (ContentEncoding::Default, b"raw\r\n\xff\0", b"raw\r\n\xff\0"),
    (ContentEncoding::SevenBit, b"raw\n\x80", b"raw\n\x80"),
    (ContentEncoding::EightBit, b"x\xff", b"x\xff"),
    (ContentEncoding::Binary, b"\0\xff\r\n", b"\0\xff\r\n"),
    (ContentEncoding::Base64, b"Q2F0\r\n", b"Cat"),
    (
        ContentEncoding::QuotedPrintable,
        b"Cat=20nap=\r\n=0A=FF",
        b"Cat nap\n\xff",
    ),
    (
        ContentEncoding::UuEncode,
        b"ignored\r\nbegin 644 cat\r\n#0V%T\r\n`\r\nend\r\n",
        b"Cat",
    ),
];

#[test]
fn construction_is_lazy_and_raw_copy_matches_mimekit() {
    let mut source = Source::new(&[1, 2, 3], usize::MAX);
    source.cursor.set_position(2);
    let mut content = MimeContent::new(source);
    assert_eq!(content.encoding(), ContentEncoding::Default);
    assert_eq!(content.new_line_format(), None);
    assert_eq!(content.get_ref().cursor.position(), 2);
    assert_eq!(content.get_ref().reads, 0);
    assert!(content.get_ref().seeks.is_empty());

    let mut sink = Sink::new(usize::MAX);
    sink.bytes.push(9);
    assert_eq!(content.write_to(&mut sink).unwrap(), 3);
    assert_eq!(sink.bytes, [9, 1, 2, 3]);
    assert_eq!(sink.flushes, 0);
    assert_eq!(content.get_ref().cursor.position(), 0);
    assert_eq!(
        content.get_ref().seeks,
        [SeekFrom::Start(0), SeekFrom::Start(0)]
    );
}

#[test]
fn every_encoding_supports_raw_copy_decoded_copy_and_open() {
    for &(encoding, stored, expected) in CASES {
        for source_size in [1, 2, 7, 4096] {
            for sink_size in [1, 3, usize::MAX] {
                let source = Source::new(stored, source_size);
                let mut content = MimeContent::with_encoding(source, encoding);
                content.get_mut().cursor.set_position(stored.len() as u64);
                let mut raw = Sink::new(sink_size);
                assert_eq!(content.write_to(&mut raw).unwrap(), stored.len() as u64);
                assert_eq!(raw.bytes, stored);
                assert_eq!(raw.flushes, 0);
                assert_eq!(content.get_ref().cursor.position(), 0);

                for _ in 0..2 {
                    let mut decoded = Sink::new(sink_size);
                    decoded.bytes.push(b'!');
                    assert_eq!(
                        content.decode_to(&mut decoded).unwrap(),
                        expected.len() as u64
                    );
                    assert_eq!(decoded.bytes[0], b'!');
                    assert_eq!(&decoded.bytes[1..], expected);
                    assert_eq!(decoded.flushes, 1);
                    assert_eq!(content.get_ref().cursor.position(), 0);
                }

                let mut decoded = Vec::new();
                content.open().unwrap().read_to_end(&mut decoded).unwrap();
                assert_eq!(decoded, expected);
                assert_eq!(content.get_ref().cursor.position(), stored.len() as u64);
                assert_eq!(content.encoding(), encoding);
            }
        }
    }
}

#[test]
fn opening_is_lazy_and_partial_reader_drop_does_not_rewind() {
    let source = Source::new(b"Q2F0Q", usize::MAX);
    let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
    content.get_mut().cursor.set_position(3);
    {
        let mut reader = content.open().unwrap();
        assert_eq!(reader.get_ref().cursor.position(), 0);
        assert_eq!(reader.get_ref().reads, 0);
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        assert_eq!(reader.get_ref().reads, 0);
        assert_eq!(reader.fill_buf().unwrap(), b"Cat");
        reader.consume(1);
        assert_eq!(reader.buffer(), b"at");
    }
    assert_eq!(content.get_ref().cursor.position(), 5);
    assert_eq!(content.get_ref().seeks, [SeekFrom::Start(0)]);

    let mut decoded = Vec::new();
    content.open().unwrap().read_to_end(&mut decoded).unwrap();
    assert_eq!(decoded, b"Cat");
    assert_eq!(content.get_ref().seeks.len(), 2);
}

#[test]
fn opened_reader_preserves_partial_codec_state_across_read_errors() {
    let mut source = Source::new(b"Q2F0", usize::MAX);
    source.steps = VecDeque::from([Step::Limit(2), Step::Error(ErrorKind::WouldBlock)]);
    let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
    let mut reader = content.open().unwrap();
    let mut buffer = [0xaa; 3];
    assert_eq!(
        reader.read(&mut buffer).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    assert_eq!(buffer, [0xaa; 3]);
    assert_eq!(reader.get_ref().cursor.position(), 2);
    reader.read_exact(&mut buffer).unwrap();
    assert_eq!(&buffer, b"Cat");
}

#[test]
fn large_binary_bodies_cross_multiple_source_buffers() {
    let expected: Vec<u8> = (0..=255)
        .cycle()
        .filter(|byte| !matches!(byte, b'\r' | b'\n'))
        .take(32768)
        .collect();
    for encoding in [
        ContentEncoding::Base64,
        ContentEncoding::QuotedPrintable,
        ContentEncoding::UuEncode,
    ] {
        let mut stored = if encoding == ContentEncoding::UuEncode {
            b"begin 644 data\r\n".to_vec()
        } else {
            Vec::new()
        };
        let mut encoder = encoder_filter(encoding);
        encoder.filter(&expected, &mut stored);
        encoder.finish(&mut stored);
        if encoding == ContentEncoding::UuEncode {
            stored.extend_from_slice(b"end\r\n");
        }
        for source_size in [1, 11, 4096] {
            let mut content =
                MimeContent::with_encoding(Source::new(&stored, source_size), encoding);
            let mut sink = Sink::new(7);
            assert_eq!(content.decode_to(&mut sink).unwrap(), expected.len() as u64);
            assert_eq!(sink.bytes, expected);
            assert_eq!(content.get_ref().cursor.position(), 0);
        }
    }
}

#[test]
fn empty_bodies_and_incomplete_payloads_keep_decoder_tolerance() {
    for &(encoding, _, _) in CASES {
        let mut content = MimeContent::with_encoding(Source::new(b"", 1), encoding);
        let mut sink = Sink::new(0);
        assert_eq!(content.write_to(&mut sink).unwrap(), 0);
        assert_eq!(sink.flushes, 0);
        assert_eq!(content.decode_to(&mut sink).unwrap(), 0);
        assert_eq!(sink.flushes, 1);
        assert!(sink.bytes.is_empty());
        assert_eq!(content.get_ref().seeks.len(), 4);
    }
    for (encoding, stored, expected) in [
        (ContentEncoding::Base64, &b"Q"[..], &b""[..]),
        (ContentEncoding::QuotedPrintable, &b"abc="[..], &b"abc"[..]),
        (ContentEncoding::UuEncode, &b"#0V%T\n"[..], &b""[..]),
    ] {
        let mut content = MimeContent::with_encoding(Cursor::new(stored), encoding);
        for _ in 0..2 {
            let mut output = Vec::new();
            assert_eq!(
                content.decode_to(&mut output).unwrap(),
                expected.len() as u64
            );
            assert_eq!(output, expected);
        }
    }
}

#[test]
fn copy_retries_interrupted_io_and_handles_short_io() {
    for decode in [false, true] {
        let mut source = Source::new(b"Q2F0Q2F0", 2);
        source.steps = VecDeque::from([
            Step::Error(ErrorKind::Interrupted),
            Step::Limit(1),
            Step::Error(ErrorKind::Interrupted),
        ]);
        let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
        let mut sink = Sink::new(1);
        sink.steps = VecDeque::from([
            Step::Error(ErrorKind::Interrupted),
            Step::Limit(1),
            Step::Error(ErrorKind::Interrupted),
        ]);
        let expected: &[u8] = if decode { b"CatCat" } else { b"Q2F0Q2F0" };
        assert_eq!(
            copy(&mut content, &mut sink, decode).unwrap(),
            expected.len() as u64
        );
        assert_eq!(sink.bytes, expected);
        assert_eq!(sink.flushes, usize::from(decode));
        assert_eq!(content.get_ref().cursor.position(), 0);
    }
}

#[test]
fn initial_rewind_failures_precede_any_read_write_or_flush() {
    for decode in [false, true] {
        let mut source = Source::new(b"Q2F0", 1);
        source.cursor.set_position(2);
        source.fail_seek = Some(1);
        let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
        let mut sink = Sink::new(1);
        let error = copy(&mut content, &mut sink, decode).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "source rewind failure");
        assert_eq!(content.get_ref().reads, 0);
        assert_eq!(content.get_ref().cursor.position(), 2);
        assert!(sink.bytes.is_empty());
        assert_eq!(sink.flushes, 0);
    }
    let mut source = Source::new(b"Q2F0", 1);
    source.fail_seek = Some(1);
    let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
    assert_eq!(
        content.open().err().unwrap().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(content.get_ref().reads, 0);
}

#[test]
fn read_failure_retains_partial_output_and_does_not_attempt_rewind() {
    for decode in [false, true] {
        let mut source = Source::new(b"Q2F0Q2F0", usize::MAX);
        source.steps = VecDeque::from([Step::Limit(4), Step::Error(ErrorKind::PermissionDenied)]);
        // A cleanup seek must not obscure the read error with another error.
        source.fail_seek = Some(2);
        let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
        let mut sink = Sink::new(usize::MAX);
        let error = copy(&mut content, &mut sink, decode).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "source read failure");
        assert_eq!(sink.bytes, if decode { &b"Cat"[..] } else { &b"Q2F0"[..] });
        assert_eq!(sink.flushes, 0);
        assert_eq!(content.get_ref().cursor.position(), 4);
        assert_eq!(content.get_ref().seeks.len(), 1);
    }
}

#[test]
fn write_failure_is_not_resumed_by_repeating_a_copy() {
    for decode in [false, true] {
        let source = Source::new(b"Q2F0Q2F0", usize::MAX);
        let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
        let mut sink = Sink::new(usize::MAX);
        sink.steps = VecDeque::from([Step::Limit(1), Step::Error(ErrorKind::BrokenPipe)]);
        assert_eq!(
            copy(&mut content, &mut sink, decode).unwrap_err().kind(),
            ErrorKind::BrokenPipe
        );
        assert_eq!(sink.bytes.len(), 1);
        assert_eq!(sink.flushes, 0);
        assert_eq!(content.get_ref().cursor.position(), 8);
        assert_eq!(content.get_ref().seeks.len(), 1);

        let expected: &[u8] = if decode { b"CatCat" } else { b"Q2F0Q2F0" };
        assert_eq!(
            copy(&mut content, &mut sink, decode).unwrap(),
            expected.len() as u64
        );
        // Content operations restart from zero; they cannot undo sink side effects.
        assert_eq!(sink.bytes[0], expected[0]);
        assert_eq!(&sink.bytes[1..], expected);
        assert_eq!(content.get_ref().cursor.position(), 0);
    }
}

#[test]
fn zero_byte_sink_writes_are_errors_not_successful_truncation() {
    for decode in [false, true] {
        let source = Source::new(b"Q2F0", usize::MAX);
        let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
        let mut sink = Sink::new(0);
        assert_eq!(
            copy(&mut content, &mut sink, decode).unwrap_err().kind(),
            ErrorKind::WriteZero
        );
        assert!(sink.bytes.is_empty());
        assert_eq!(sink.flushes, 0);
        assert_eq!(content.get_ref().seeks.len(), 1);
    }
}

#[test]
fn final_rewind_failure_reports_error_after_complete_output_without_flush() {
    for decode in [false, true] {
        let mut source = Source::new(b"Q2F0", usize::MAX);
        source.fail_seek = Some(2);
        let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
        let mut sink = Sink::new(usize::MAX);
        assert_eq!(
            copy(&mut content, &mut sink, decode).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
        assert_eq!(sink.bytes, if decode { &b"Cat"[..] } else { &b"Q2F0"[..] });
        assert_eq!(sink.flushes, 0);
        assert_eq!(content.get_ref().cursor.position(), 4);
        assert_eq!(content.get_ref().seeks.len(), 2);
    }
}

#[test]
fn only_decoded_copy_flushes_and_flush_failure_leaves_source_rewound() {
    let source = Source::new(b"Q2F0", usize::MAX);
    let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
    let mut sink = Sink::new(usize::MAX);
    sink.flush_error = Some(ErrorKind::BrokenPipe);
    assert_eq!(content.write_to(&mut sink).unwrap(), 4);
    assert_eq!(sink.flushes, 0);
    sink.bytes.clear();
    assert_eq!(
        content.decode_to(&mut sink).unwrap_err().kind(),
        ErrorKind::BrokenPipe
    );
    assert_eq!(sink.bytes, b"Cat");
    assert_eq!(sink.flushes, 1);
    assert_eq!(content.get_ref().cursor.position(), 0);
    // Retrying just the sink flush does not copy the body a second time.
    sink.flush().unwrap();
    assert_eq!(sink.bytes, b"Cat");
}

#[test]
fn newline_hint_is_metadata_only_and_borrowed_source_is_recoverable() {
    let mut source = Cursor::new(b"Q2F0\r\n");
    let mut content = MimeContent::with_encoding(&mut source, ContentEncoding::Base64);
    for hint in [
        Some(NewLineFormat::Dos),
        Some(NewLineFormat::Unix),
        Some(NewLineFormat::Mixed),
        None,
    ] {
        content.set_new_line_format(hint);
        assert_eq!(content.new_line_format(), hint);
        let mut stored = Vec::new();
        content.write_to(&mut stored).unwrap();
        assert_eq!(stored, b"Q2F0\r\n");
        let mut decoded = Vec::new();
        content.decode_to(&mut decoded).unwrap();
        assert_eq!(decoded, b"Cat");
        assert_eq!(content.new_line_format(), hint);
    }
    content.get_mut().set_position(2);
    let recovered = content.into_inner();
    assert_eq!(recovered.position(), 2);
    let mut rest = Vec::new();
    recovered.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, b"F0\r\n");
    assert_eq!(source.position(), 6);
}

#[test]
fn decoded_copy_flushes_but_does_not_finalize_a_downstream_encoder() {
    let mut content =
        MimeContent::with_encoding(Cursor::new(b"aGVsbG8=\r\n"), ContentEncoding::Base64);
    let mut output = Vec::new();
    let mut writer = FilteredWriter::new(&mut output, encoder_filter(ContentEncoding::Base64));
    assert_eq!(content.decode_to(&mut writer).unwrap(), 5);
    assert_eq!(writer.get_ref().as_slice(), b"aGVs");
    writer.finish().unwrap();
    assert_eq!(output, b"aGVsbG8=\r\n");
}

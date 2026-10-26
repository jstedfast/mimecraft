// Adapted from MimeKit's UnitTests/Encodings/YEncTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Write},
};

use mimecraft::{
    charsets::Charset,
    encodings::{
        EncoderOptionsError, NewLineFormat, YDecoder, YEncHeader, YEncHeaderError, YEncWriteError,
        YEncWriter, YEncWriterOptions,
    },
};

const VECTOR: &[u8] = b"123456789";
const EMPTY_BLOCK: &[u8] =
    b"=ybegin line=128 size=0 name=empty.bin\r\n=yend size=0 crc32=00000000\r\n";
const VECTOR_BLOCK: &[u8] =
    b"=ybegin line=128 size=9 name=test.bin\r\n[\\]^_`abc\r\n=yend size=9 crc32=cbf43926\r\n";

struct NoRead;

impl Read for NoRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("preflight or faulted writes must not read the source")
    }
}

struct Chunked<'a> {
    data: &'a [u8],
    chunk: usize,
    interrupt: bool,
}

impl Read for Chunked<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(output.len() <= 4096);
        self.interrupt = !self.interrupt;
        if self.interrupt {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = self.data.len().min(self.chunk).min(output.len());
        output[..count].copy_from_slice(&self.data[..count]);
        self.data = &self.data[count..];
        Ok(count)
    }
}

struct Sink {
    data: Vec<u8>,
    chunk: usize,
    interrupt: bool,
    fail_at: Option<usize>,
    zero: bool,
}

impl Sink {
    fn new(chunk: usize) -> Self {
        Self {
            data: Vec::new(),
            chunk,
            interrupt: false,
            fail_at: None,
            zero: false,
        }
    }
}

impl Write for Sink {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.interrupt = !self.interrupt;
        if self.interrupt {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.zero {
            return Ok(0);
        }
        let mut count = input.len().min(self.chunk);
        if let Some(limit) = self.fail_at {
            if self.data.len() == limit {
                return Err(io::Error::other("sink sentinel"));
            }
            count = count.min(limit - self.data.len());
        }
        self.data.extend_from_slice(&input[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        panic!("block writing and dropping must not flush")
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 0 { 0 } else { 0xedb8_8320 };
        }
    }
    !crc
}

fn payload(wire: &[u8]) -> &[u8] {
    let start = wire.iter().position(|&b| b == b'\n').unwrap() + 1;
    let end = wire
        .windows(6)
        .rposition(|window| window == b"=yend ")
        .unwrap();
    &wire[start..end]
}

#[test]
fn checked_metadata() {
    let header = YEncHeader::new("file name.bin", 20).unwrap();
    assert_eq!(header.file_name(), "file name.bin");
    assert_eq!(header.size(), 20);
    assert_eq!(header.decoded_size(), 20);
    assert_eq!(header.line_length(), 128);
    assert_eq!(header.part_number(), None);
    assert_eq!(header.total_parts(), None);
    assert_eq!(header.range(), None);
    let part = header
        .clone()
        .with_part(2, Some(3), 10..=18)
        .unwrap()
        .with_line_length(60)
        .unwrap();
    assert_eq!(part.size(), 20);
    assert_eq!(part.decoded_size(), 9);
    assert_eq!(part.line_length(), 60);
    assert_eq!(part.part_number(), Some(2));
    assert_eq!(part.total_parts(), Some(3));
    assert_eq!(part.range(), Some(&(10..=18)));
    assert_eq!(header.part_number(), None);
    assert_eq!(
        header
            .clone()
            .with_part(1, None, 1..=1)
            .unwrap()
            .decoded_size(),
        1
    );
    assert_eq!(
        YEncHeader::new("x", u64::MAX)
            .unwrap()
            .with_part(u32::MAX, Some(u32::MAX), 1..=u64::MAX)
            .unwrap()
            .decoded_size(),
        u64::MAX
    );
    assert_eq!(YEncHeader::new("x", 0).unwrap().decoded_size(), 0);

    for name in ["", " x", "x ", " ", "x\0y", "x\ry", "x\ny"] {
        assert_eq!(
            YEncHeader::new(name, 0).unwrap_err(),
            YEncHeaderError::InvalidFileName
        );
    }
    for name in ["../x", "a=b c", "\tx\t", "\u{a0}"] {
        assert!(YEncHeader::new(name, 0).is_ok());
    }
    for length in [0, i32::MAX as usize - 1, usize::MAX] {
        assert_eq!(
            header.clone().with_line_length(length).unwrap_err(),
            YEncHeaderError::InvalidLineLength(length)
        );
    }
    for length in [1, 60, 998, i32::MAX as usize - 2] {
        assert_eq!(
            header
                .clone()
                .with_line_length(length)
                .unwrap()
                .line_length(),
            length
        );
    }
    assert!(matches!(
        header.clone().with_part(0, None, 1..=1),
        Err(YEncHeaderError::InvalidPartNumber(0))
    ));
    for total in [0, 1] {
        assert!(matches!(
            header.clone().with_part(2, Some(total), 1..=1),
            Err(YEncHeaderError::InvalidTotalParts { .. })
        ));
    }
    for (begin, end) in [(0, 1), (2, 1), (1, 21), (21, 21)] {
        assert!(matches!(
            header.clone().with_part(1, None, begin..=end),
            Err(YEncHeaderError::InvalidRange { .. })
        ));
    }
    assert!(
        YEncHeader::new("x", 0)
            .unwrap()
            .with_part(1, None, 1..=1)
            .is_err()
    );
}

#[test]
fn upstream_wire_formats_and_reuse() {
    for (format, newline) in [(NewLineFormat::Dos, "\r\n"), (NewLineFormat::Unix, "\n")] {
        let options = YEncWriterOptions {
            new_line_format: format,
            ..Default::default()
        };
        let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
        assert_eq!(
            writer
                .write_block(
                    &YEncHeader::new("empty.bin", 0).unwrap(),
                    &mut &b""[..],
                    None
                )
                .unwrap(),
            0
        );
        let mut source = Cursor::new(b"prefix123456789");
        source.set_position(6);
        assert_eq!(
            writer
                .write_block(
                    &YEncHeader::new("test.bin", 9).unwrap(),
                    &mut source,
                    Some(0xcbf43926)
                )
                .unwrap(),
            0xcbf43926
        );
        assert_eq!(source.position(), 15);
        let expected = String::from_utf8([EMPTY_BLOCK, VECTOR_BLOCK].concat())
            .unwrap()
            .replace("\r\n", newline);
        assert_eq!(writer.get_ref(), expected.as_bytes());
        assert!(!writer.is_faulted());

        let header = YEncHeader::new("test.bin", 20)
            .unwrap()
            .with_part(2, Some(3), 10..=18)
            .unwrap();
        let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
        assert_eq!(
            writer
                .write_block(&header, &mut &VECTOR[..], Some(0x12345678))
                .unwrap(),
            0xcbf43926
        );
        let expected = "=ybegin part=2 total=3 line=128 size=20 name=test.bin\r\n\
            =ypart begin=10 end=18\r\n[\\]^_`abc\r\n\
            =yend size=9 part=2 total=3 pcrc32=cbf43926 crc32=12345678\r\n";
        assert_eq!(
            writer.into_inner(),
            expected.replace("\r\n", newline).as_bytes()
        );
    }
}

#[test]
fn chunked_round_trips_and_finalized_checksums() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for length in [0, 1, 256, 4095, 4096, 4097, 16385] {
            let data: Vec<_> = (0..length)
                .map(|i: usize| i.wrapping_add(214) as u8)
                .collect();
            for chunk in [1, 17, 4096] {
                let mut source = Chunked {
                    data: &data,
                    chunk,
                    interrupt: false,
                };
                let mut sink = Sink::new(chunk);
                let options = YEncWriterOptions {
                    new_line_format: format,
                    max_line_length: 61,
                    ..Default::default()
                };
                let header = YEncHeader::new("file name.bin", length as u64)
                    .unwrap()
                    .with_line_length(60)
                    .unwrap();
                {
                    let mut writer = YEncWriter::with_options(&mut sink, &options).unwrap();
                    assert_eq!(
                        writer.write_block(&header, &mut source, None).unwrap(),
                        crc32(&data)
                    );
                }
                let mut decoder = YDecoder::for_payload();
                let mut decoded = Vec::new();
                for bytes in payload(&sink.data).chunks(chunk) {
                    decoder.decode(bytes, &mut decoded);
                }
                assert_eq!(decoded, data);
                assert_eq!(!decoder.checksum(), crc32(&data));
                assert!(source.data.is_empty());
                for line in sink.data.split(|&byte| byte == b'\n') {
                    assert!(line.strip_suffix(b"\r").unwrap_or(line).len() <= 61);
                }
                if format == NewLineFormat::Unix {
                    assert!(!sink.data.contains(&b'\r'));
                }
            }
        }
    }
}

#[test]
fn every_supported_payload_line_length_reserves_the_escape_byte() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for length in 60..=998 {
            let mut data = vec![1; length - 1];
            data.push(214);
            let header = YEncHeader::new("x", data.len() as u64)
                .unwrap()
                .with_line_length(length)
                .unwrap();
            let options = YEncWriterOptions {
                max_line_length: length + 1,
                new_line_format: format,
                ..Default::default()
            };
            let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
            writer
                .write_block(&header, &mut data.as_slice(), None)
                .unwrap();
            let line = payload(writer.get_ref());
            let line = line.strip_suffix(b"\n").unwrap();
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            assert_eq!(line.len(), length + 1);
            assert!(line.ends_with(b"=@"));
        }
    }
}

#[test]
fn short_long_and_checksum_failures_fault_the_writer() {
    for (size, checksum) in [(8, None), (10, None), (9, Some(0))] {
        let mut writer = YEncWriter::new(Vec::new());
        let header = YEncHeader::new("x", size).unwrap();
        let mut source = Cursor::new(VECTOR);
        let error = writer
            .write_block(&header, &mut source, checksum)
            .unwrap_err();
        match size {
            8 => assert!(matches!(
                error,
                YEncWriteError::SourceTooLong { expected: 8 }
            )),
            10 => assert!(matches!(
                error,
                YEncWriteError::SourceTooShort {
                    expected: 10,
                    actual: 9
                }
            )),
            _ => assert!(matches!(
                error,
                YEncWriteError::ChecksumMismatch {
                    expected: 0,
                    actual: 0xcbf43926
                }
            )),
        }
        assert_eq!(source.position(), 9);
        assert!(!writer.get_ref().windows(6).any(|w| w == b"=yend "));
        assert!(writer.is_faulted());
        let before = writer.get_ref().clone();
        assert!(matches!(
            writer.write_block(&header, &mut NoRead, None),
            Err(YEncWriteError::Faulted)
        ));
        assert_eq!(writer.get_ref(), &before);
    }
    for size in [0, 4095, 4096, 4097] {
        let data = vec![0; size + 20];
        let mut source = Cursor::new(data);
        let header = YEncHeader::new("x", size as u64).unwrap();
        let mut writer = YEncWriter::new(Vec::new());
        assert!(matches!(
            writer.write_block(&header, &mut source, None),
            Err(YEncWriteError::SourceTooLong { .. })
        ));
        assert_eq!(source.position(), size as u64 + 1);
    }
}

#[test]
fn multipart_optional_totals_and_whole_file_verification() {
    for (size, range, checksum, succeeds) in [
        (9, 1..=9, Some(0xcbf43926), true),
        (9, 1..=9, Some(0), false),
        (20, 1..=9, Some(0), true),
        (20, 12..=20, None, true),
    ] {
        let header = YEncHeader::new("test.bin", size)
            .unwrap()
            .with_part(1, None, range)
            .unwrap();
        let mut writer = YEncWriter::new(Vec::new());
        let result = writer.write_block(&header, &mut &VECTOR[..], checksum);
        assert_eq!(result.is_ok(), succeeds);
        let text = String::from_utf8(writer.into_inner()).unwrap();
        assert!(!text.contains("total="));
        if succeeds {
            assert!(text.contains("=yend size=9 part=1 pcrc32=cbf43926"));
            assert_eq!(text.contains(" crc32="), checksum.is_some());
        } else {
            assert!(matches!(
                result,
                Err(YEncWriteError::ChecksumMismatch { .. })
            ));
            assert!(!text.contains("=yend "));
        }
    }
}

#[test]
fn options_limits_and_preflight_reusability() {
    for limit in [0, i32::MAX as usize - 1, usize::MAX] {
        let options = YEncWriterOptions {
            max_line_length: limit,
            ..Default::default()
        };
        assert!(matches!(YEncWriter::with_options(Vec::new(), &options),
            Err(YEncWriteError::InvalidMaxLineLength(value)) if value == limit));
    }
    let options = YEncWriterOptions {
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    assert!(matches!(
        YEncWriter::with_options(Vec::new(), &options),
        Err(YEncWriteError::Encoder(
            EncoderOptionsError::InvalidNewLineFormat(NewLineFormat::Mixed)
        ))
    ));

    let options = YEncWriterOptions {
        max_line_length: 61,
        max_decoded_size: 0,
        ..Default::default()
    };
    let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
    for length in [1, 59, 999, i32::MAX as usize - 2] {
        let header = YEncHeader::new("x", 0)
            .unwrap()
            .with_line_length(length)
            .unwrap();
        assert!(matches!(
            writer.write_block(&header, &mut NoRead, None),
            Err(YEncWriteError::Encoder(
                EncoderOptionsError::InvalidLineLength(_)
            ))
        ));
    }
    let header = YEncHeader::new("x", 0)
        .unwrap()
        .with_line_length(61)
        .unwrap();
    assert!(matches!(
        writer.write_block(&header, &mut NoRead, None),
        Err(YEncWriteError::LineTooLong {
            required: 62,
            limit: 61
        })
    ));
    let header = YEncHeader::new("x", 1)
        .unwrap()
        .with_line_length(60)
        .unwrap();
    assert!(matches!(
        writer.write_block(&header, &mut NoRead, None),
        Err(YEncWriteError::BlockTooLarge { size: 1, limit: 0 })
    ));
    assert!(writer.get_ref().is_empty());
    assert!(!writer.is_faulted());
    writer
        .write_block(
            &YEncHeader::new("x", 0)
                .unwrap()
                .with_line_length(60)
                .unwrap(),
            &mut &b""[..],
            None,
        )
        .unwrap();

    let options = YEncWriterOptions {
        max_decoded_size: u64::MAX,
        ..options
    };
    let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
    let header = YEncHeader::new("x", 1_000_000_000_000_000_000)
        .unwrap()
        .with_line_length(60)
        .unwrap()
        .with_part(1, None, 1..=1_000_000_000_000_000_000)
        .unwrap();
    let required = "=yend size=1000000000000000000 part=1 pcrc32=00000000 crc32=00000000".len();
    assert!(matches!(writer.write_block(&header, &mut NoRead, Some(0)),
        Err(YEncWriteError::LineTooLong { required: value, limit: 61 }) if value == required));
    assert!(writer.get_ref().is_empty());
    assert!(!writer.is_faulted());

    for limit in [required - 1, required] {
        let options = YEncWriterOptions {
            max_line_length: limit,
            ..options
        };
        let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
        if limit < required {
            assert!(matches!(
                writer.write_block(&header, &mut NoRead, Some(0)),
                Err(YEncWriteError::LineTooLong { required: actual, limit: actual_limit })
                    if actual == required && actual_limit == limit
            ));
            assert!(writer.get_ref().is_empty());
            assert!(!writer.is_faulted());
        } else {
            assert!(matches!(
                writer.write_block(&header, &mut &b""[..], Some(0)),
                Err(YEncWriteError::SourceTooShort {
                    expected: 1_000_000_000_000_000_000,
                    actual: 0
                })
            ));
            assert!(!writer.get_ref().is_empty());
            assert!(writer.is_faulted());
        }
    }

    let options = YEncWriterOptions {
        max_decoded_size: 9,
        ..Default::default()
    };
    let header = YEncHeader::new("x", u64::MAX)
        .unwrap()
        .with_part(1, None, 1..=9)
        .unwrap();
    let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
    assert_eq!(
        writer.write_block(&header, &mut &VECTOR[..], None).unwrap(),
        0xcbf43926
    );
}

#[test]
fn filename_charsets_and_snapshot() {
    for (charset, name, encoded) in [
        (Charset::LATIN1, "caf\u{e9}.bin", b"caf\xe9.bin".as_slice()),
        (
            Charset::UTF8,
            "\u{65e5}\u{672c}\u{8a9e}.bin",
            b"\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e.bin",
        ),
    ] {
        let mut options = YEncWriterOptions {
            file_name_charset: charset,
            ..Default::default()
        };
        let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
        options.file_name_charset = Charset::ASCII;
        assert_ne!(options.file_name_charset, charset);
        writer
            .write_block(&YEncHeader::new(name, 0).unwrap(), &mut &b""[..], None)
            .unwrap();
        let expected = [
            b"=ybegin line=128 size=0 name=".as_slice(),
            encoded,
            b"\r\n=yend size=0 crc32=00000000\r\n",
        ]
        .concat();
        assert_eq!(writer.into_inner(), expected);
    }
    for (charset, name) in [(Charset::ASCII, "caf\u{e9}"), (Charset::LATIN1, "\u{65e5}")] {
        let options = YEncWriterOptions {
            file_name_charset: charset,
            ..Default::default()
        };
        let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
        let error = writer
            .write_block(&YEncHeader::new(name, 0).unwrap(), &mut NoRead, None)
            .unwrap_err();
        assert!(matches!(error, YEncWriteError::FileNameEncoding(_)));
        assert!(error.source().is_some());
        assert!(writer.get_ref().is_empty());
        assert!(!writer.is_faulted());
    }
    let options = YEncWriterOptions {
        file_name_charset: Charset::from_label("utf-16le").unwrap(),
        ..Default::default()
    };
    let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
    for (name, expected_offset) in [("x", 1), ("\u{100}", 0), ("\u{10a}", 0), ("\u{10d}", 0)] {
        assert!(
            matches!(writer.write_block(&YEncHeader::new(name, 0).unwrap(), &mut NoRead, None),
            Err(YEncWriteError::InvalidEncodedFileName { offset }) if offset == expected_offset)
        );
        assert!(writer.get_ref().is_empty());
        assert!(!writer.is_faulted());
    }
    writer
        .write_block(
            &YEncHeader::new("\u{6162}", 0).unwrap(),
            &mut &b""[..],
            None,
        )
        .unwrap();
    assert!(
        writer
            .get_ref()
            .starts_with(b"=ybegin line=128 size=0 name=ba\r\n")
    );
}

#[test]
fn encoded_filename_line_limit_counts_bytes() {
    let name = "\u{e9}".repeat(20);
    let required = b"=ybegin line=60 size=0 name=".len() + name.len();
    let header = YEncHeader::new(&name, 0)
        .unwrap()
        .with_line_length(60)
        .unwrap();
    for limit in [required - 1, required] {
        let options = YEncWriterOptions {
            max_line_length: limit,
            file_name_charset: Charset::UTF8,
            ..Default::default()
        };
        let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
        if limit < required {
            assert!(matches!(writer.write_block(&header, &mut NoRead, None),
                Err(YEncWriteError::LineTooLong { required: actual, limit: actual_limit })
                    if actual == required && actual_limit == limit));
            assert!(writer.get_ref().is_empty());
            assert!(!writer.is_faulted());
        } else {
            writer.write_block(&header, &mut &b""[..], None).unwrap();
            assert_eq!(
                writer.get_ref().iter().position(|&b| b == b'\r'),
                Some(required)
            );
        }
    }
}

#[test]
fn destination_failures_at_each_stage_are_preserved_and_latched() {
    let header = YEncHeader::new("test.bin", 9).unwrap();
    let header_len = VECTOR_BLOCK.iter().position(|&b| b == b'\n').unwrap() + 1;
    for fail_at in [
        0,
        5,
        header_len,
        header_len + 4,
        header_len + 9,
        header_len + 11,
        VECTOR_BLOCK.len() - 1,
    ] {
        let mut sink = Sink::new(7);
        sink.fail_at = Some(fail_at);
        let mut writer = YEncWriter::new(sink);
        let error = writer
            .write_block(&header, &mut &VECTOR[..], None)
            .unwrap_err();
        assert!(
            matches!(&error, YEncWriteError::Io(error) if error.kind() == io::ErrorKind::Other)
        );
        assert_eq!(error.source().unwrap().to_string(), "sink sentinel");
        assert!(writer.is_faulted());
        assert_eq!(writer.get_ref().data, VECTOR_BLOCK[..fail_at]);
        assert!(matches!(
            writer.write_block(&header, &mut NoRead, None),
            Err(YEncWriteError::Faulted)
        ));
    }
    let mut writer = YEncWriter::new(Sink::new(1));
    writer.get_mut().zero = true;
    assert!(
        matches!(writer.write_block(&header, &mut &VECTOR[..], None),
        Err(YEncWriteError::Io(error)) if error.kind() == io::ErrorKind::WriteZero)
    );
    assert!(writer.is_faulted());
}

struct FailingSource(usize);

impl Read for FailingSource {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.0 == 0 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "source sentinel"));
        }
        let n = self.0.min(output.len());
        output[..n].fill(b'1');
        self.0 -= n;
        Ok(n)
    }
}

#[test]
fn source_failures_are_preserved_and_latched() {
    let header = YEncHeader::new("test.bin", 9).unwrap();
    for length in [0, 4, 9] {
        let mut writer = YEncWriter::new(Vec::new());
        let error = writer
            .write_block(&header, &mut FailingSource(length), None)
            .unwrap_err();
        assert!(
            matches!(&error, YEncWriteError::Io(error) if error.kind() == io::ErrorKind::TimedOut)
        );
        assert_eq!(error.source().unwrap().to_string(), "source sentinel");
        assert!(writer.is_faulted());
        assert!(!writer.get_ref().windows(6).any(|w| w == b"=yend "));
        assert!(matches!(
            writer.write_block(&header, &mut NoRead, None),
            Err(YEncWriteError::Faulted)
        ));
    }
}

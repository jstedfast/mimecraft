// Adapted from MimeKit's UnitTests/Encodings/YEncTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Read, Write},
};

use mimecraft::{
    charsets::Charset,
    encodings::{
        NewLineFormat, YEncHeader, YEncReadError, YEncReader, YEncReaderOptions, YEncWriter,
        YEncWriterOptions,
    },
};

const EMPTY: &[u8] = b"=ybegin line=128 size=0 name=empty.bin\r\n=yend size=0 crc32=00000000\r\n";
const VECTOR: &[u8] =
    b"=ybegin line=128 size=9 name=test.bin\r\n[\\]^_`abc\r\n=yend size=9 crc32=cbf43926\r\n";

struct Chunked<'a> {
    data: &'a [u8],
    chunk: usize,
    reads: usize,
    interrupt: bool,
    eof: bool,
}

impl<'a> Chunked<'a> {
    fn new(data: &'a [u8], chunk: usize) -> Self {
        Self {
            data,
            chunk,
            reads: 0,
            interrupt: false,
            eof: false,
        }
    }
}

impl Read for Chunked<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(!self.eof, "read after EOF");
        assert!(output.len() <= 4096);
        self.reads += 1;
        self.interrupt = !self.interrupt;
        if self.interrupt {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = self.chunk.min(output.len()).min(self.data.len());
        output[..count].copy_from_slice(&self.data[..count]);
        self.data = &self.data[count..];
        self.eof = count == 0;
        Ok(count)
    }
}

struct NoWrite;

impl Write for NoWrite {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        panic!("state errors and invalid payload lines must not write")
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("decoding must not flush")
    }
}

fn assert_faulted(reader: &mut YEncReader<Chunked<'_>>) {
    let reads = reader.get_ref().reads;
    assert!(reader.is_faulted());
    assert!(reader.checksum().is_none());
    assert!(reader.expected_checksum().is_none());
    assert!(reader.expected_part_checksum().is_none());
    assert!(matches!(reader.read_next(), Err(YEncReadError::Faulted)));
    assert!(matches!(
        reader.decode_to(&mut NoWrite),
        Err(YEncReadError::Faulted)
    ));
    assert_eq!(reader.get_ref().reads, reads);
}

#[test]
fn upstream_valid_blocks() {
    let cases = [
        "=ybegin line=128 size=0 name=empty.bin\n=yend size=0".to_owned(),
        "=ybegin line=128 size=0 name=empty.bin\r\n=yend size=0\r".to_owned(),
        "=ybegin  future=1 line=128 size=0 name=  empty name=1.bin  \n=yend size=0 crc32=00000000".to_owned(),
        "=ybegin line=128 size=9 name=test.bin\n[\\]^_`abc\n=yend size=9 crc32=CBF43926".to_owned(),
        "=ybegin part=1 line=128 size=9 name=test.bin\n=ypart begin=1 end=9\n[\\]^_`abc\n=yend size=9 part=1".to_owned(),
        "=ybegin part=1 total=1 line=128 size=9 name=test.bin\n=ypart begin=1 end=9\n[\\]^_`abc\n=yend size=9 part=1 total=1 pcrc32=cbf43926 crc32=cbf43926".to_owned(),
        format!("=ybegin line=60 size=60 name=test.bin\n{}=@\n=yend size=60", "*".repeat(59)),
    ];
    for text in cases {
        for chunk in [1, 7, 4096] {
            let mut reader = YEncReader::new(Chunked::new(text.as_bytes(), chunk));
            assert!(reader.read_next().unwrap(), "{text:?}");
            assert!(reader.checksum().is_none());
            assert_eq!(reader.decoded_size(), 0);
            reader.decode_to(&mut io::sink()).unwrap();
            assert_eq!(
                reader.decoded_size(),
                reader.header().unwrap().decoded_size()
            );
            assert!(reader.checksum().is_some());
            assert!(!reader.read_next().unwrap());
        }
    }
}

#[test]
fn upstream_invalid_blocks() {
    let cases = [
        "=ybegin line=128 size=0",
        "=ybegin line=128 size=0 size=0 name=x\n",
        "=ybegin line=128 size=-1 name=x\n",
        "=ybegin line=128 size=9223372036854775808 name=x\n",
        "=ybegin line=2147483648 size=0 name=x\n",
        "=ybegin line=0 size=0 name=x\n",
        "=ybegin line= size=0 name=x\n",
        "=ybegin line=128 invalid size=0 name=x\n",
        "=ybegin line=128 size=0 name=\n",
        "=ybegin line=128 size=0 name=a\0b\n",
        "=ybegin part=0 line=128 size=1 name=x\n=ypart begin=1 end=1\n",
        "=ybegin part=2 total=1 line=128 size=1 name=x\n=ypart begin=1 end=1\n",
        "=ybegin part=1 line=128 size=1 name=x\n",
        "=ybegin part=1 line=128 size=1 name=x\n=yend size=0\n",
        "=ybegin part=1 line=128 size=1 name=x\n=ypart begin=0 end=1\n",
        "=ybegin part=1 line=128 size=1 name=x\n=ypart begin=1 end=2\n",
        "=ybegin line=128 size=0 name=x\n",
        "=ybegin line=128 size=0 name=x\n=yend\n",
        "=ybegin line=128 size=0 name=x\n=ybegin line=128 size=0 name=y\n",
        "=ybegin line=128 size=0 name=x\n=yend size=1\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 crc32=00000001\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 pcrc32=00000001\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 crc32=0\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 crc32=gggggggg\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 name=x crc32=00000001\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 name=x size=1\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 part=1\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 total=1\n",
        "=ybegin line=128 size=1 name=x\n=yend size=1\n",
        "=ybegin line=128 size=1 name=x\n**\n=yend size=2\n",
        "=ybegin line=128 size=1 name=x\n=\n=yend size=1\n",
        "=ybegin line=128 size=1 name=x\n=\0\n=yend size=1\n",
        "=ybegin line=128 size=1 name=x\n\0\n=yend size=1\n",
        "=ybegin line=128 size=1 name=x\n\r*\n=yend size=1\n",
        "=ybegin line=1 size=2 name=x\n**\n=yend size=2\n",
        "=ybegin line=2 size=2 name=x\n==*\n=yend size=2\n",
        "=ybegin part=1 line=128 size=1 name=x\n=ypart begin=1 end=1\n*\n=yend size=1\n",
        "=ybegin part=1 line=128 size=1 name=x\n=ypart begin=1 end=1\n*\n=yend size=1 part=2\n",
        "=ybegin part=1 line=128 size=1 name=x\n=ypart begin=1 end=1\n*\n=yend size=1 part=1 crc32=00000000\n",
    ];
    for (index, text) in cases.iter().enumerate() {
        for chunk in [1, 17, 4096] {
            let mut reader = YEncReader::new(Chunked::new(text.as_bytes(), chunk));
            let result = reader.read_next().and_then(|found| {
                assert!(found);
                reader.decode_to(&mut io::sink())
            });
            assert!(result.is_err(), "case {index}, chunk {chunk}");
            assert!(!result.unwrap_err().to_string().is_empty());
            assert_faulted(&mut reader);
        }
    }
}

#[test]
fn multiple_blocks_state_and_repeated_eof() {
    let input = [
        b"preamble\r\n=ybegin2 ignored\r\n".as_slice(),
        EMPTY,
        b"interstitial\n",
        VECTOR,
        b"trailing",
    ]
    .concat();
    for chunk in [1, 7, 4096] {
        let mut reader = YEncReader::new(Chunked::new(&input, chunk));
        assert!(reader.header().is_none());
        assert!(matches!(
            reader.decode_to(&mut NoWrite),
            Err(YEncReadError::NoPendingBlock)
        ));
        assert!(!reader.is_faulted());
        let mut output = Vec::new();
        for (size, checksum, name) in [(0, 0, "empty.bin"), (9, 0xcbf43926, "test.bin")] {
            assert!(reader.read_next().unwrap());
            assert_eq!(reader.header().unwrap().file_name(), name);
            assert_eq!(reader.header().unwrap().size(), size);
            assert_eq!(reader.decoded_size(), 0);
            assert!(reader.checksum().is_none());
            assert!(reader.expected_checksum().is_none());
            let reads = reader.get_ref().reads;
            assert!(matches!(
                reader.read_next(),
                Err(YEncReadError::PendingBlock)
            ));
            assert_eq!(reader.get_ref().reads, reads);
            assert!(!reader.is_faulted());
            reader.decode_to(&mut output).unwrap();
            assert_eq!(reader.decoded_size(), size);
            assert_eq!(reader.checksum(), Some(checksum));
            assert_eq!(reader.expected_checksum(), Some(checksum));
            assert_eq!(reader.expected_part_checksum(), None);
            assert!(matches!(
                reader.decode_to(&mut NoWrite),
                Err(YEncReadError::NoPendingBlock)
            ));
            assert!(!reader.is_faulted());
        }
        assert_eq!(output, b"123456789");
        assert!(!reader.read_next().unwrap());
        assert!(reader.header().is_none());
        assert_eq!(reader.decoded_size(), 0);
        assert!(reader.checksum().is_none());
        let reads = reader.get_ref().reads;
        assert!(!reader.read_next().unwrap());
        assert!(!reader.read_next().unwrap());
        assert_eq!(reader.get_ref().reads, reads);
    }
}

#[test]
fn read_ahead_is_recoverable_and_borrowed_streams_remain_open() {
    let input = [EMPTY, VECTOR].concat();
    let mut source = input.as_slice();
    {
        let mut reader = YEncReader::new(&mut source);
        reader.read_next().unwrap();
        reader.decode_to(&mut NoWrite).unwrap();
        let (source, buffered) = reader.into_parts();
        let mut remaining = buffered;
        source.read_to_end(&mut remaining).unwrap();
        assert_eq!(remaining, VECTOR);
    }
    assert!(source.is_empty());
    let mut source = Chunked::new(&input, 1);
    {
        let mut reader = YEncReader::new(&mut source);
        assert!(reader.read_next().unwrap());
    }
    let mut tail = Vec::new();
    source.read_to_end(&mut tail).unwrap();
    assert!(tail.starts_with(b"=yend "));
}

#[test]
fn physical_line_limits_cover_preamble_headers_payload_trailers_and_eof() {
    let options = YEncReaderOptions {
        max_line_length: 128,
        max_decoded_size: 9,
        ..Default::default()
    };
    for chunk in [1, 127, 128, 129, 4096] {
        for newline in [b"\n".as_slice(), b"\r\n"] {
            let input = [vec![b'x'; 128], newline.to_vec(), VECTOR.to_vec()].concat();
            let mut reader =
                YEncReader::with_options(Chunked::new(&input, chunk), &options).unwrap();
            assert!(reader.read_next().unwrap());
            reader.decode_to(&mut io::sink()).unwrap();
        }
        for input in [
            [vec![b'x'; 129], b"\r\n".to_vec()].concat(),
            [vec![b'x'; 128], b"\rx".to_vec()].concat(),
            vec![b'x'; 129],
            String::from_utf8(VECTOR.to_vec())
                .unwrap()
                .replace("size=9", "size=10")
                .into_bytes(),
            String::from_utf8(EMPTY.to_vec())
                .unwrap()
                .replace("line=128", "line=129")
                .into_bytes(),
        ] {
            let mut reader =
                YEncReader::with_options(Chunked::new(&input, chunk), &options).unwrap();
            assert!(reader.read_next().is_err());
            assert!(reader.header().is_none());
            assert_faulted(&mut reader);
        }
        for end in [b"".as_slice(), b"\r"] {
            let input = [vec![b'x'; 128], end.to_vec()].concat();
            let mut reader =
                YEncReader::with_options(Chunked::new(&input, chunk), &options).unwrap();
            assert!(!reader.read_next().unwrap());
        }
    }
    for limit in [0, i32::MAX as usize - 1, usize::MAX] {
        let options = YEncReaderOptions {
            max_line_length: limit,
            ..Default::default()
        };
        assert!(matches!(YEncReader::with_options(io::empty(), &options),
            Err(YEncReadError::InvalidMaxLineLength(actual)) if actual == limit));
    }
    let mut reader = YEncReader::with_options(io::repeat(b'x'), &options).unwrap();
    assert!(matches!(
        reader.read_next(),
        Err(YEncReadError::LineTooLong { limit: 128 })
    ));
    let options = YEncReaderOptions {
        max_decoded_size: 0,
        ..Default::default()
    };
    let mut reader = YEncReader::with_options(EMPTY, &options).unwrap();
    assert!(reader.read_next().unwrap());
    reader.decode_to(&mut NoWrite).unwrap();

    for (stage, input) in [
        (
            "header",
            format!(
                "=ybegin line=60 size=0 name={}\n=yend size=0\n",
                "x".repeat(40)
            ),
        ),
        (
            "payload",
            format!(
                "=ybegin line=60 size=62 name=x\n{}\n=yend size=62\n",
                "*".repeat(62)
            ),
        ),
        (
            "trailer",
            format!(
                "=ybegin line=60 size=0 name=x\n=yend size=0 future={}\n",
                "x".repeat(50)
            ),
        ),
    ] {
        let options = YEncReaderOptions {
            max_line_length: 61,
            ..Default::default()
        };
        let mut reader =
            YEncReader::with_options(Chunked::new(input.as_bytes(), 1), &options).unwrap();
        let error = reader
            .read_next()
            .and_then(|_| reader.decode_to(&mut NoWrite))
            .unwrap_err();
        assert!(
            matches!(error, YEncReadError::LineTooLong { limit: 61 }),
            "{stage}: {error}"
        );
        assert_faulted(&mut reader);
    }
}

#[test]
fn filename_charsets_snapshot_and_space_only_trimming() {
    for (charset, name, encoded) in [
        (Charset::LATIN1, "caf\u{e9}.bin", b"caf\xe9.bin".as_slice()),
        (
            Charset::UTF8,
            "\u{65e5}\u{672c}\u{8a9e}.bin",
            b"\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e.bin",
        ),
        (Charset::LATIN1, "\tx\t", b"\tx\t"),
        (Charset::LATIN1, "\u{a0}", b"\xa0"),
        (Charset::LATIN1, "name=x  extra=y", b"name=x  extra=y"),
    ] {
        let input = [
            b"=ybegin line=128 size=0 name=  ".as_slice(),
            encoded,
            b"  \n=yend size=0\n",
        ]
        .concat();
        let mut options = YEncReaderOptions {
            file_name_charset: charset,
            ..Default::default()
        };
        let mut reader = YEncReader::with_options(Chunked::new(&input, 1), &options).unwrap();
        options.file_name_charset = Charset::ASCII;
        assert_ne!(options.file_name_charset, charset);
        assert!(reader.read_next().unwrap());
        assert_eq!(reader.header().unwrap().file_name(), name);
        reader.decode_to(&mut NoWrite).unwrap();
    }
    let options = YEncReaderOptions {
        file_name_charset: Charset::UTF8,
        ..Default::default()
    };
    let mut reader = YEncReader::with_options(
        Chunked::new(b"=ybegin line=128 size=0 name=\xff\n=yend size=0\n", 1),
        &options,
    )
    .unwrap();
    let error = reader.read_next().unwrap_err();
    assert!(matches!(error, YEncReadError::FileNameDecoding(_)));
    assert!(error.source().is_some());
    assert_faulted(&mut reader);
}

#[test]
fn control_fields_are_case_sensitive_checked_and_forward_compatible() {
    let valid = b"=ybegin  extension=yes line=000128 size=0000 total=0 name=x\n\
                  =yend size=0 part=0 total=0 pcrc32=00000000 other=ignored\n";
    let mut reader = YEncReader::new(valid.as_slice());
    assert!(reader.read_next().unwrap());
    assert!(reader.header().unwrap().part_number().is_none());
    reader.decode_to(&mut NoWrite).unwrap();
    assert_eq!(reader.expected_part_checksum(), Some(0));
    for text in [
        "=ybegin LINE=128 size=0 name=x\n",
        "=ybegin line=128 SIZE=0 name=x\n",
        "=ybegin line=128 size=0 NAME=x\n",
        "=ybegin line=+128 size=0 name=x\n",
        "=ybegin line=128 size=18446744073709551616 name=x\n",
        "=ybegin part=4294967296 line=128 size=1 name=x\n=ypart begin=1 end=1\n",
        "=ybegin part=1 total=4294967296 line=128 size=1 name=x\n=ypart begin=1 end=1\n",
        "=ybegin line=128 total=1 size=0 name=x\n",
        "=ybegin line=128 size=0 unknown=a unknown=b name=x\n",
        "=ybegin =bad line=128 size=0 name=x\n",
        "=ybegin x= line=128 size=0 name=x\n",
        "=ybegin line=128\tsize=0 name=x\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 x=a x=b\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 crc32=+0000000\n",
        "=ybegin line=128 size=0 name=x\n=yend size=0 crc32=0000000g\n",
    ] {
        let mut reader = YEncReader::new(Chunked::new(text.as_bytes(), 1));
        assert!(
            reader
                .read_next()
                .and_then(|_| reader.decode_to(&mut NoWrite))
                .is_err(),
            "{text:?}"
        );
        assert_faulted(&mut reader);
    }
    let input = b"=ybegin x=1 x=2 line=128 size=0 name=x\n";
    let mut reader = YEncReader::new(input.as_slice());
    assert!(matches!(
        reader.read_next(),
        Err(YEncReadError::DuplicateField { offset: 12 })
    ));
}

#[test]
fn unsigned_metadata_and_per_part_size_limits() {
    let header = YEncHeader::new("x", u64::MAX)
        .unwrap()
        .with_part(u32::MAX, Some(u32::MAX), u64::MAX - 8..=u64::MAX)
        .unwrap();
    let mut writer = YEncWriter::new(Vec::new());
    writer
        .write_block(&header, &mut &b"123456789"[..], Some(0))
        .unwrap();
    let options = YEncReaderOptions {
        max_decoded_size: 9,
        ..Default::default()
    };
    let mut reader = YEncReader::with_options(writer.get_ref().as_slice(), &options).unwrap();
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.header(), Some(&header));
    reader.decode_to(&mut io::sink()).unwrap();
    assert_eq!(reader.decoded_size(), 9);
    assert_eq!(reader.expected_checksum(), Some(0)); // Unverifiable partial-file CRC.
    assert_eq!(reader.expected_part_checksum(), Some(0xcbf43926));
    let options = YEncReaderOptions {
        max_decoded_size: 8,
        ..options
    };
    let mut reader = YEncReader::with_options(writer.get_ref().as_slice(), &options).unwrap();
    assert!(matches!(
        reader.read_next(),
        Err(YEncReadError::BlockTooLarge { size: 9, limit: 8 })
    ));

    let input = b"=ybegin line=128 size=18446744073709551615 name=x\n";
    let options = YEncReaderOptions {
        max_decoded_size: u64::MAX,
        ..Default::default()
    };
    let mut reader = YEncReader::with_options(input.as_slice(), &options).unwrap();
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.header().unwrap().decoded_size(), u64::MAX);
    assert!(matches!(
        reader.decode_to(&mut NoWrite),
        Err(YEncReadError::MissingTrailer)
    ));
}

#[test]
fn exact_payload_grammar_and_line_end_escape_exception() {
    for (width, encoded, decoded) in [
        (1, b"=@".as_slice(), [214].as_slice()),
        (2, b"A==", &[23, 211]),
        (2, b"\t.", &[223, 4]),
    ] {
        let input = [
            format!("=ybegin line={width} size={} name=x\n", decoded.len()).into_bytes(),
            encoded.to_vec(),
            format!("\n=yend size={}\n", decoded.len()).into_bytes(),
        ]
        .concat();
        let mut reader = YEncReader::new(Chunked::new(&input, 1));
        assert!(reader.read_next().unwrap());
        let mut output = Vec::new();
        reader.decode_to(&mut output).unwrap();
        assert_eq!(output, decoded);
    }
    for (encoded, escape) in [
        (b"A\0".as_slice(), false),
        (b"A\rB", false),
        (b"A=", true),
        (b"A=\0", true),
        (b"A=\rB", true),
    ] {
        let input = [
            b"=ybegin line=128 size=9 name=x\n".as_slice(),
            encoded,
            b"\n=yend size=9",
        ]
        .concat();
        let mut reader = YEncReader::new(Chunked::new(&input, 1));
        reader.read_next().unwrap();
        let error = reader.decode_to(&mut NoWrite).unwrap_err();
        if escape {
            assert!(matches!(error, YEncReadError::InvalidEscape { offset: 1 }));
        } else {
            assert!(matches!(
                error,
                YEncReadError::InvalidPayloadByte { offset: 1 }
            ));
        }
        assert_eq!(reader.decoded_size(), 0);
        assert_faulted(&mut reader);
    }
    let mut reader =
        YEncReader::new(b"=ybegin line=128 size=1 name=x\n**\n=yend size=2\n".as_slice());
    reader.read_next().unwrap();
    assert!(matches!(
        reader.decode_to(&mut NoWrite),
        Err(YEncReadError::PayloadTooLarge { expected: 1 })
    ));
    assert_eq!(reader.decoded_size(), 0);
}

#[test]
fn checksum_validation_and_publication_are_atomic() {
    for (size, begin, end, checksum, part_checksum, succeeds) in [
        (9, 1, 9, Some(0xcbf43926), Some(0xcbf43926), true),
        (9, 1, 9, Some(0), Some(0xcbf43926), false),
        (20, 10, 18, Some(0), Some(0xcbf43926), true),
        (20, 10, 18, Some(0), Some(0), false),
        (20, 10, 18, None, None, true),
    ] {
        let mut input = format!(
            "=ybegin part=2 total=3 line=128 size={size} name=x\n\
             =ypart begin={begin} end={end}\n[\\]^_`abc\n=yend size=9 part=2"
        );
        if let Some(checksum) = checksum {
            input.push_str(&format!(" crc32={checksum:08x}"));
        }
        if let Some(checksum) = part_checksum {
            input.push_str(&format!(" pcrc32={checksum:08x}"));
        }
        let mut reader = YEncReader::new(Chunked::new(input.as_bytes(), 1));
        reader.read_next().unwrap();
        let mut output = Vec::new();
        let result = reader.decode_to(&mut output);
        assert_eq!(output, b"123456789");
        assert_eq!(reader.decoded_size(), 9);
        assert_eq!(result.is_ok(), succeeds);
        if succeeds {
            assert_eq!(reader.checksum(), Some(0xcbf43926));
            assert_eq!(reader.expected_checksum(), checksum);
            assert_eq!(reader.expected_part_checksum(), part_checksum);
        } else {
            assert!(matches!(
                result,
                Err(YEncReadError::ChecksumMismatch {
                    actual: 0xcbf43926,
                    ..
                })
            ));
            assert_faulted(&mut reader);
        }
    }
}

struct FailingSource<'a> {
    data: &'a [u8],
    remaining: usize,
}

impl Read for FailingSource<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "source sentinel"));
        }
        let count = output.len().min(self.data.len()).min(self.remaining);
        output[..count].copy_from_slice(&self.data[..count]);
        self.data = &self.data[count..];
        self.remaining -= count;
        Ok(count)
    }
}

struct Sink {
    output: Vec<u8>,
    remaining: Option<usize>,
    zero: bool,
    interrupt: bool,
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
        if self.remaining == Some(0) {
            return Err(io::Error::other("sink sentinel"));
        }
        let count = input.len().min(1);
        self.output.extend_from_slice(&input[..count]);
        if let Some(remaining) = &mut self.remaining {
            *remaining -= count;
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("reader must not flush the borrowed destination")
    }
}

#[test]
fn source_and_destination_failures_latch_without_publishing_checksums() {
    for remaining in [0, 10, 37, 42, VECTOR.len() - 1] {
        let source = FailingSource {
            data: VECTOR,
            remaining,
        };
        let mut reader = YEncReader::new(source);
        let result = reader
            .read_next()
            .and_then(|_| reader.decode_to(&mut io::sink()));
        let error = result.unwrap_err();
        assert!(
            matches!(&error, YEncReadError::Io(error) if error.kind() == io::ErrorKind::TimedOut)
        );
        assert_eq!(error.source().unwrap().to_string(), "source sentinel");
        assert!(reader.is_faulted());
        assert!(reader.checksum().is_none());
        assert!(matches!(reader.read_next(), Err(YEncReadError::Faulted)));
        assert!(matches!(
            reader.decode_to(&mut NoWrite),
            Err(YEncReadError::Faulted)
        ));
    }
    for (remaining, zero) in [
        (None, false),
        (Some(0), false),
        (Some(4), false),
        (None, true),
    ] {
        let mut reader = YEncReader::new(Chunked::new(VECTOR, 1));
        reader.read_next().unwrap();
        let mut sink = Sink {
            output: Vec::new(),
            remaining,
            zero,
            interrupt: false,
        };
        let result = reader.decode_to(&mut sink);
        if remaining.is_none() && !zero {
            result.unwrap();
            assert_eq!(sink.output, b"123456789");
        } else {
            let error = result.unwrap_err();
            if zero {
                assert!(
                    matches!(&error, YEncReadError::Io(error) if error.kind() == io::ErrorKind::WriteZero)
                );
            } else {
                assert_eq!(error.source().unwrap().to_string(), "sink sentinel");
                assert_eq!(sink.output, &b"123456789"[..remaining.unwrap()]);
            }
            assert_eq!(reader.decoded_size(), 9);
            assert_faulted(&mut reader);
        }
    }
}

#[test]
fn complete_writer_reader_round_trips_for_all_widths_and_initial_bytes() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for width in 60..=998 {
            let mut data = vec![1; width - 1];
            data.push(214);
            let header = YEncHeader::new("x", data.len() as u64)
                .unwrap()
                .with_line_length(width)
                .unwrap();
            let options = YEncWriterOptions {
                max_line_length: width + 1,
                new_line_format: format,
                ..Default::default()
            };
            let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
            let checksum = writer
                .write_block(&header, &mut data.as_slice(), None)
                .unwrap();
            let options = YEncReaderOptions {
                max_line_length: width + 1,
                ..Default::default()
            };
            let mut reader =
                YEncReader::with_options(writer.get_ref().as_slice(), &options).unwrap();
            assert!(reader.read_next().unwrap());
            let mut output = Vec::new();
            reader.decode_to(&mut output).unwrap();
            assert_eq!(output, data);
            assert_eq!(reader.checksum(), Some(checksum));
            assert_eq!(reader.expected_checksum(), Some(checksum));
        }
        for first in 0..=255 {
            let options = YEncWriterOptions {
                new_line_format: format,
                ..Default::default()
            };
            let mut writer = YEncWriter::with_options(Vec::new(), &options).unwrap();
            writer
                .write_block(
                    &YEncHeader::new("x", 1).unwrap(),
                    &mut [first].as_slice(),
                    None,
                )
                .unwrap();
            let mut reader = YEncReader::new(Chunked::new(writer.get_ref(), 1));
            assert!(reader.read_next().unwrap());
            let mut output = Vec::new();
            reader.decode_to(&mut output).unwrap();
            assert_eq!(output, [first]);
        }
    }
}

#[test]
fn truncation_and_byte_mutations_never_silently_validate_bad_checksums() {
    for end in 0..=VECTOR.len() {
        let mut reader = YEncReader::new(Chunked::new(&VECTOR[..end], 1));
        let result = reader.read_next().and_then(|found| {
            if found {
                reader.decode_to(&mut io::sink()).map(|_| true)
            } else {
                Ok(false)
            }
        });
        if matches!(result, Ok(true)) {
            assert_eq!(reader.checksum(), Some(0xcbf43926));
            assert_eq!(reader.decoded_size(), 9);
        } else if result.is_err() {
            assert_faulted(&mut reader);
        }
    }
    for index in 0..VECTOR.len() {
        for byte in [0, b'\r', b'\n', b'=', b' ', b'9', 255] {
            let mut input = VECTOR.to_vec();
            input[index] = byte;
            let mut reader = YEncReader::new(Chunked::new(&input, 7));
            let mut output = Vec::new();
            let result = reader.read_next().and_then(|found| {
                if found {
                    reader.decode_to(&mut output).map(|_| true)
                } else {
                    Ok(false)
                }
            });
            if matches!(result, Ok(true)) {
                assert_eq!(output, b"123456789");
                assert_eq!(reader.checksum(), Some(0xcbf43926));
            } else if result.is_err() {
                assert_faulted(&mut reader);
            }
        }
    }
}

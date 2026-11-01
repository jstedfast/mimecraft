// Adapted from MimeKit's UnitTests/Tnef/RtfCompressedWriteStreamTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::support as rtf_compressed_workloads;
use super::{Builder, Mode, SIMPLE, SIMPLE_TEXT, check, crc, random};
use mimecraft::tnef::RtfCompressedWriter as Writer;
use std::{
    cell::Cell,
    io::{self, Cursor, Seek, SeekFrom, Write},
    rc::Rc,
};

#[path = "../../benches/support/rtf_compressed_writer.rs"]
mod workloads;

fn compress(input: &[u8], mode: Mode, chunk: usize) -> Vec<u8> {
    let mut writer = Writer::new(Cursor::new(Vec::new()), mode).unwrap();
    for bytes in input.chunks(chunk) {
        writer.write_all(bytes).unwrap();
    }
    assert_eq!(writer.raw_size(), input.len() as u64);
    assert_eq!(writer.compression_mode(), mode);
    assert!(!writer.is_finished());
    writer.finish().unwrap();
    assert!(writer.is_finished());
    assert!(!writer.is_faulted());
    let end = writer.get_ref().position();
    writer.finish().unwrap();
    writer.flush().unwrap();
    assert_eq!(writer.get_ref().position(), end);
    assert_eq!(
        writer.write(b"x").unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(!writer.is_faulted());
    assert_eq!(writer.write(&[]).unwrap(), 0);
    let bytes = writer.into_inner().into_inner();
    assert_eq!(
        u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
        bytes.len() - 4
    );
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
        input.len()
    );
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        u32::from(mode)
    );
    assert_eq!(
        u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        if mode == Mode::Compressed {
            crc(&bytes[16..])
        } else {
            0
        }
    );
    check(&bytes, input, mode);
    bytes
}

#[test]
fn spec_example_without_repetition_is_byte_exact() {
    // [MS-OXRTFCP] 4.1: Compressing Plain Text Without Repetition.
    for chunk in [1, 7, 4096, usize::MAX] {
        assert_eq!(compress(SIMPLE_TEXT, Mode::Compressed, chunk), SIMPLE);
    }
}

#[test]
fn spec_example_with_repetition_overlaps_the_write_position() {
    // [MS-OXRTFCP] 4.2: The match overlaps bytes it produces.
    let input = b"{\\rtf1 WXYZWXYZWXYZWXYZWXYZ}";
    let bytes = compress(input, Mode::Compressed, usize::MAX);
    // The overlapping match makes the output much smaller than the input.
    assert!(bytes.len() - 16 < input.len() - 8);
}

#[test]
fn spec_uncompressed_and_empty_modes() {
    // [MS-OXRTFCP] 4.3: Uncompressed RTF has a zero CRC.
    let input = b"{\\rtf1\\ansi\\ansicpg1252\\pard test}";
    let bytes = compress(input, Mode::Uncompressed, 7);
    assert_eq!(bytes.len(), input.len() + 16);
    assert_eq!(&bytes[16..], input);
    let mut builder = Builder::new();
    builder.end();
    assert_eq!(compress(&[], Mode::Compressed, 1), builder.build());
    assert_eq!(compress(&[], Mode::Uncompressed, 1).len(), 16);
}

#[test]
fn large_repetitive_input_preserves_ratio_and_chunk_invariance() {
    let input = workloads::workload("Repetitive").input;
    let expected = compress(&input, Mode::Compressed, usize::MAX);
    assert!(expected.len() < input.len() / 3);
    for chunk in [1, 7, 4096] {
        assert_eq!(compress(&input, Mode::Compressed, chunk), expected);
    }
}

#[test]
fn random_and_small_alphabet_inputs_cross_dictionary_wraps() {
    for seed in 0..4 {
        let mut state = seed + 1;
        // Use a small alphabet for some inputs to exercise short/overlapping matches.
        let input: Vec<_> = (0..20000 + seed * 1234)
            .map(|_| {
                let value = random(&mut state);
                if seed % 2 == 0 {
                    value as u8
                } else {
                    b'a' + (value % 3) as u8
                }
            })
            .collect();
        let expected = compress(&input, Mode::Compressed, usize::MAX);
        assert_eq!(compress(&input, Mode::Compressed, 13), expected);
        compress(&input, Mode::Uncompressed, usize::MAX);
    }
}

#[test]
fn single_byte_runs_cover_short_matches_and_tail_lengths() {
    for length in 1..64 {
        compress(&vec![b'x'; length], Mode::Compressed, 1);
    }
    for length in [3888, 3889, 4095, 4096, 4097, 8192, 16384] {
        compress(&vec![0; length], Mode::Compressed, 7);
    }
}

#[test]
fn every_split_and_intermediate_flush_preserves_wire_output() {
    for split in 0..=SIMPLE_TEXT.len() {
        let mut writer = Writer::new(Cursor::new(Vec::new()), Mode::Compressed).unwrap();
        writer.write_all(&SIMPLE_TEXT[..split]).unwrap();
        writer.flush().unwrap();
        assert!(!writer.is_finished());
        assert_eq!(&writer.get_ref().get_ref()[..16], &[0; 16]);
        writer.write_all(&[]).unwrap();
        writer.write_all(&SIMPLE_TEXT[split..]).unwrap();
        writer.flush().unwrap();
        writer.finish().unwrap();
        assert_eq!(writer.into_inner().into_inner(), SIMPLE);
    }
    let input = workloads::workload("Overlapping").input;
    let expected = compress(&input, Mode::Compressed, usize::MAX);
    let mut writer = Writer::new(Cursor::new(Vec::new()), Mode::Compressed).unwrap();
    for bytes in input.chunks(7) {
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
    }
    writer.finish().unwrap();
    assert_eq!(writer.into_inner().into_inner(), expected);
}

#[test]
fn nonzero_positions_and_sequential_streams_preserve_other_bytes() {
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        let expected = compress(SIMPLE_TEXT, mode, 1);
        let mut sink = Cursor::new(vec![0xff; 3 + expected.len() + 5]);
        sink.set_position(3);
        let mut writer = Writer::new(&mut sink, mode).unwrap();
        writer.write_all(SIMPLE_TEXT).unwrap();
        writer.finish().unwrap();
        drop(writer);
        assert_eq!(sink.position(), (3 + expected.len()) as u64);
        assert_eq!(&sink.get_ref()[..3], &[0xff; 3]);
        assert_eq!(&sink.get_ref()[3..3 + expected.len()], expected);
        assert_eq!(&sink.get_ref()[3 + expected.len()..], &[0xff; 5]);
        let mut writer = Writer::new(&mut sink, mode).unwrap();
        writer.write_all(SIMPLE_TEXT).unwrap();
        writer.finish().unwrap();
        drop(writer);
        assert_eq!(&sink.get_ref()[3 + expected.len()..], expected);
    }
}

#[derive(Clone, Copy)]
enum Failure {
    Write(usize),
    Seek(usize),
    Flush,
    Zero,
    InterruptWrite,
    InterruptSeek,
    InterruptFlush,
}

struct Sink {
    output: Cursor<Vec<u8>>,
    failure: Rc<Cell<Option<Failure>>>,
    calls: Rc<Cell<usize>>,
    chunk: usize,
}

impl Default for Sink {
    fn default() -> Self {
        Self {
            output: Cursor::new(Vec::new()),
            failure: Rc::new(Cell::new(None)),
            calls: Rc::new(Cell::new(0)),
            chunk: usize::MAX,
        }
    }
}

impl Write for Sink {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.calls.set(self.calls.get() + 1);
        let mut length = input.len().min(self.chunk);
        match self.failure.get() {
            Some(Failure::Write(0)) => return Err(io::Error::other("write failure")),
            Some(Failure::Write(remaining)) => {
                length = length.min(remaining);
                self.failure.set(Some(Failure::Write(remaining - length)));
            }
            Some(Failure::Zero) => return Ok(0),
            Some(Failure::InterruptWrite) => {
                self.failure.set(None);
                return Err(io::ErrorKind::Interrupted.into());
            }
            _ => {}
        }
        self.output.write(&input[..length])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.calls.set(self.calls.get() + 1);
        match self.failure.get() {
            Some(Failure::Flush) => return Err(io::Error::other("flush failure")),
            Some(Failure::InterruptFlush) => {
                self.failure.set(None);
                return Err(io::ErrorKind::Interrupted.into());
            }
            _ => {}
        }
        Ok(())
    }
}

impl Seek for Sink {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.calls.set(self.calls.get() + 1);
        match self.failure.get() {
            Some(Failure::Seek(0)) => return Err(io::Error::other("seek failure")),
            Some(Failure::Seek(remaining)) => self.failure.set(Some(Failure::Seek(remaining - 1))),
            Some(Failure::InterruptSeek) => {
                self.failure.set(None);
                return Err(io::ErrorKind::Interrupted.into());
            }
            _ => {}
        }
        self.output.seek(position)
    }
}

fn assert_faulted(writer: &mut Writer<Sink>) {
    assert!(writer.is_faulted());
    assert!(!writer.is_finished());
    let calls = writer.get_ref().calls.get();
    assert_eq!(
        writer.write(b"x").unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        writer.write(&[]).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        writer.flush().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        writer.finish().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(writer.get_ref().calls.get(), calls);
}

#[test]
fn short_and_interrupted_io_preserves_all_input_and_header_bytes() {
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        let expected = compress(SIMPLE_TEXT, mode, 1);
        for chunk in [1, 7] {
            let sink = Sink {
                chunk,
                ..Default::default()
            };
            let control = Rc::clone(&sink.failure);
            control.set(Some(Failure::InterruptSeek));
            let mut writer = Writer::new(sink, mode).unwrap();
            control.set(Some(Failure::InterruptWrite));
            writer.write_all(SIMPLE_TEXT).unwrap();
            writer.flush().unwrap();
            control.set(Some(Failure::InterruptSeek));
            writer.finish().unwrap();
            control.set(Some(Failure::InterruptFlush));
            writer.flush().unwrap();
            assert_eq!(writer.into_inner().output.into_inner(), expected);
        }
    }
}

#[test]
fn constructor_rejects_unknown_modes_before_io_and_reports_reservation_failures() {
    for mode in [0, 42, 0x75465a4c, u32::MAX] {
        let mut sink = Sink::default();
        assert_eq!(
            Writer::new(&mut sink, Mode::Unknown(mode))
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(sink.calls.get(), 0);
    }
    for failure in [Failure::Seek(0), Failure::Write(3), Failure::Zero] {
        let mut sink = Sink::default();
        sink.failure.set(Some(failure));
        assert!(Writer::new(&mut sink, Mode::Compressed).is_err());
        assert!(sink.output.get_ref().len() <= 3);
    }
}

#[test]
fn encode_failures_never_acknowledge_new_input() {
    for failure in [Failure::Write(3), Failure::Zero] {
        let sink = Sink::default();
        let control = Rc::clone(&sink.failure);
        let mut writer = Writer::new(sink, Mode::Compressed).unwrap();
        assert_eq!(writer.write(&[b'x'; 5000]).unwrap(), 4096);
        assert_eq!(writer.raw_size(), 4096);
        control.set(Some(failure));
        assert!(writer.write(b"next input").is_err());
        assert_eq!(writer.raw_size(), 4096);
        assert_faulted(&mut writer);
    }
    let sink = Sink::default();
    let control = Rc::clone(&sink.failure);
    let mut writer = Writer::new(sink, Mode::Uncompressed).unwrap();
    control.set(Some(Failure::Write(3)));
    assert_eq!(writer.write(b"abcdef").unwrap(), 3);
    assert_eq!(writer.raw_size(), 3);
    assert!(writer.write(b"def").is_err());
    assert_eq!(writer.raw_size(), 3);
    assert_faulted(&mut writer);
}

#[test]
fn finalization_write_position_seek_restore_and_flush_errors_are_terminal() {
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        for failure in [
            Failure::Write(0),
            Failure::Write(3),
            Failure::Zero,
            Failure::Seek(0),
            Failure::Seek(1),
            Failure::Seek(2),
            Failure::Flush,
        ] {
            let sink = Sink::default();
            let control = Rc::clone(&sink.failure);
            let mut writer = Writer::new(sink, mode).unwrap();
            writer.write_all(SIMPLE_TEXT).unwrap();
            control.set(Some(failure));
            assert!(writer.finish().is_err());
            assert_faulted(&mut writer);
        }
    }
}

#[test]
fn flush_failure_faults_without_finishing_and_drop_never_writes() {
    for finish in [false, true] {
        let sink = Sink::default();
        let control = Rc::clone(&sink.failure);
        let calls = Rc::clone(&sink.calls);
        let mut writer = Writer::new(sink, Mode::Compressed).unwrap();
        writer.write_all(SIMPLE_TEXT).unwrap();
        if finish {
            writer.finish().unwrap();
        } else {
            control.set(Some(Failure::Flush));
            assert!(writer.flush().is_err());
            assert_faulted(&mut writer);
        }
        let before = calls.get();
        drop(writer);
        assert_eq!(calls.get(), before);
    }
    let mut sink = Sink::default();
    {
        let mut writer = Writer::new(&mut sink, Mode::Compressed).unwrap();
        writer.write_all(b"not finished").unwrap();
    }
    assert_eq!(sink.output.into_inner(), vec![0; 16]);
    let sink = Sink::default();
    let calls = Rc::clone(&sink.calls);
    let mut writer = Writer::new(sink, Mode::Compressed).unwrap();
    writer.write_all(b"not finished").unwrap();
    let before = calls.get();
    let output = writer.into_inner();
    assert_eq!(calls.get(), before);
    assert_eq!(output.output.into_inner(), vec![0; 16]);
}

#[test]
fn writer_benchmarks_validate_headers_crc_full_content_and_ratios() {
    for name in workloads::CASES {
        let case = workloads::workload(name);
        assert_eq!(case.name, name);
        assert!(!case.input.is_empty());
        case.check();
    }
}

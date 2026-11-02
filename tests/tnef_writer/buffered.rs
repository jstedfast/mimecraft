// Adapted from MimeKit's TnefWriterTests buffered attribute and temporary-stream cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{Tag, TnefWriter, attribute, decode, header};
use std::{
    cell::Cell,
    fs::{self, OpenOptions},
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::Path,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

fn payload(length: usize) -> Vec<u8> {
    let mut state = 42u32;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

#[test]
fn unknown_length_attribute_matches_direct_output_across_chunks() {
    let data = payload(100_000);
    let mut direct = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    direct
        .write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    direct.write_attribute(Tag::ATTACH_DATA, &data).unwrap();
    direct.finish().unwrap();
    let expected = direct.into_inner();
    for chunk in [1, 7, 2047, 2048, 4096, data.len()] {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        writer
            .write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])
            .unwrap();
        let before = writer.get_ref().clone();
        let mut stream = writer.open_attribute(Tag::ATTACH_DATA).unwrap();
        assert!(stream.is_empty());
        assert_eq!(stream.len(), 0);
        assert_eq!(stream.write(&[]).unwrap(), 0);
        for bytes in data.chunks(chunk) {
            stream.write_all(bytes).unwrap();
            stream.flush().unwrap();
        }
        assert_eq!(stream.len(), data.len() as u64);
        assert!(!stream.is_faulted());
        let buffer = stream.finish().unwrap();
        assert_eq!(buffer.len(), data.len() as u64);
        assert_eq!(buffer.position(), buffer.len());
        assert_eq!(buffer.to_vec().unwrap(), data);
        assert_eq!(&writer.get_ref()[..before.len()], &before);
        writer.finish().unwrap();
        assert_eq!(writer.into_inner(), expected);
    }
    assert_eq!(decode(&expected).last().unwrap().2, data);
}

#[test]
fn upstream_split_writes_and_empty_streams_round_trip() {
    let data = payload(100_000);
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    writer
        .write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    let mut stream = writer.open_attribute(Tag::ATTACH_DATA).unwrap();
    stream.write_all(&data[..1]).unwrap();
    stream.write_all(&data[1..1000]).unwrap();
    stream.write_all(&data[1000..]).unwrap();
    stream.flush().unwrap();
    assert_eq!(stream.len(), 100_000);
    stream.finish().unwrap();
    writer
        .open_attribute(Tag::ATTACH_META_FILE)
        .unwrap()
        .finish()
        .unwrap();
    writer.finish().unwrap();
    let values = decode(writer.get_ref());
    assert_eq!(values[3].2, data);
    assert!(values[4].2.is_empty());
}

#[test]
fn child_flush_only_flushes_buffer_and_finish_emits_before_parent_flush() {
    let output = super::Sink::new(usize::MAX);
    let output_calls = output.calls.clone();
    let buffer = Buffer::new();
    let control = buffer.control.clone();
    let mut writer = TnefWriter::new(output, 1252, 0).unwrap();
    let mut stream = writer
        .open_attribute_with_buffer(Tag::BODY, buffer)
        .unwrap();
    stream.write_all(b"body\0").unwrap();
    stream.flush().unwrap();
    assert_eq!(control.flushes.get(), 1);
    assert_eq!(output_calls.get(), 0, "buffering performed output I/O");
    let buffer = stream.finish().unwrap();
    assert!(!control.dropped.get());
    assert_eq!(buffer.inner.position(), 5);
    let mut expected = header(1252, 0);
    attribute(&mut expected, 1, Tag::BODY, b"body\0");
    assert_eq!(writer.get_ref().bytes, expected);
    drop(buffer);
    assert!(control.dropped.get());
    writer.write_string(Tag::SUBJECT, "x").unwrap();
    writer.finish().unwrap();
    assert_eq!(decode(&writer.into_inner().bytes).len(), 4);
}

#[test]
fn dropping_or_recovering_unfinished_children_performs_no_io_and_faults_parent() {
    for complete_input in [false, true] {
        for recover in [false, true] {
            let sink = super::Sink::new(usize::MAX);
            let output_calls = sink.calls.clone();
            let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
            writer.write_string(Tag::SUBJECT, "x").unwrap();
            let before = writer.get_ref().bytes.clone();
            let buffer = Buffer::new();
            let control = buffer.control.clone();
            let mut stream = writer
                .open_attribute_with_buffer(Tag::BODY, buffer)
                .unwrap();
            if complete_input {
                stream.write_all(b"body").unwrap();
            }
            let reads = control.reads.get();
            let writes = control.writes.get();
            let seeks = control.seeks.get();
            let flushes = control.flushes.get();
            let calls = output_calls.get();
            if recover {
                let buffer = stream.into_inner();
                assert!(!control.dropped.get());
                drop(buffer);
            } else {
                drop(stream);
            }
            assert!(control.dropped.get());
            assert_eq!(control.reads.get(), reads);
            assert_eq!(control.writes.get(), writes);
            assert_eq!(control.seeks.get(), seeks);
            assert_eq!(control.flushes.get(), flushes);
            assert_eq!(output_calls.get(), calls);
            assert!(writer.is_faulted());
            super::assert_faulted(&mut writer);
            assert_eq!(writer.get_ref().bytes, before);
            assert_eq!(output_calls.get(), calls);
        }
    }
}

#[test]
fn forgetting_an_empty_child_cannot_bypass_completion() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    // The empty default buffer has no heap storage to leak.
    std::mem::forget(writer.open_attribute(Tag::BODY).unwrap());
    assert!(writer.is_faulted());
    assert_eq!(
        writer.finish().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(writer.into_inner().is_empty());
}

#[test]
fn failed_opens_leave_grammar_unchanged_and_owned_buffers_are_released() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    for state in 0..5 {
        let mut buffer = Buffer::new();
        let control = buffer.control.clone();
        let expected = match state {
            0 => {
                buffer.inner.set_position(1);
                io::ErrorKind::InvalidInput
            }
            1 => {
                buffer.inner = Cursor::new(vec![1]);
                io::ErrorKind::InvalidInput
            }
            _ => {
                control.seek_failure.set(Some(state - 2));
                io::ErrorKind::Other
            }
        };
        assert_eq!(
            writer
                .open_attribute_with_buffer(Tag::MAPI_PROPERTIES, buffer)
                .unwrap_err()
                .kind(),
            expected,
        );
        assert!(control.dropped.get());
        assert!(!writer.is_faulted());
        assert!(writer.get_ref().is_empty());
    }
    writer.write_string(Tag::SUBJECT, "retry").unwrap();
    let mut properties = writer.open_attribute(Tag::MAPI_PROPERTIES).unwrap();
    properties.write_all(&[0; 4]).unwrap();
    properties.finish().unwrap();
    assert!(writer.write_string(Tag::SUBJECT, "too late").is_err());
    writer.finish().unwrap();
    assert_eq!(decode(writer.get_ref()).len(), 4);
}

#[test]
fn invalid_tags_and_states_do_not_touch_scratch_storage() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    for tag in [
        Tag::NULL,
        Tag::TNEF_VERSION,
        Tag::OEM_CODEPAGE,
        Tag(0x12345678),
        Tag::ATTACH_DATA,
    ] {
        let buffer = Buffer::new();
        let control = buffer.control.clone();
        assert_eq!(
            writer
                .open_attribute_with_buffer(tag, buffer)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(control.seeks.get(), 0);
        assert_eq!(control.writes.get(), 0);
        assert!(control.dropped.get());
    }
    writer.finish().unwrap();
    let buffer = Buffer::new();
    let control = buffer.control.clone();
    assert_eq!(
        writer
            .open_attribute_with_buffer(Tag::BODY, buffer)
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(control.seeks.get(), 0);
}

#[test]
fn short_buffer_io_and_interrupted_operations_preserve_acknowledged_length() {
    let data = payload(65537);
    for chunk in [1, 7, 4096] {
        let mut buffer = Buffer::new();
        buffer.chunk = chunk;
        let control = buffer.control.clone();
        control.interrupt_seek.set(true);
        let mut writer = TnefWriter::new(super::Sink::new(7), 1252, 0).unwrap();
        let mut stream = writer
            .open_attribute_with_buffer(Tag::BODY, buffer)
            .unwrap();
        control.interrupt_write.set(true);
        assert_eq!(stream.write(&data).unwrap(), chunk);
        assert_eq!(stream.len(), chunk as u64);
        stream.write_all(&data[chunk..]).unwrap();
        assert_eq!(stream.len(), data.len() as u64);
        control.interrupt_flush.set(true);
        control.interrupt_seek.set(true);
        control.interrupt_read.set(true);
        let buffer = stream.finish().unwrap();
        assert_eq!(buffer.inner.position(), data.len() as u64);
        writer.finish().unwrap();
        assert_eq!(decode(&writer.into_inner().bytes)[2].2, data);
    }
}

#[test]
fn buffer_write_and_zero_write_failures_are_terminal() {
    for zero in [false, true] {
        let mut buffer = Buffer::new();
        buffer.chunk = 3;
        let control = buffer.control.clone();
        control.zero_write.set(zero);
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut stream = writer
            .open_attribute_with_buffer(Tag::BODY, buffer)
            .unwrap();
        assert_eq!(stream.write(b"abcdef").unwrap(), 3);
        control.fail_write.set(true);
        assert_eq!(
            stream.write(b"def").unwrap_err().kind(),
            if zero {
                io::ErrorKind::WriteZero
            } else {
                io::ErrorKind::Other
            }
        );
        assert_eq!(stream.len(), 3);
        assert!(stream.is_faulted());
        let writes = control.writes.get();
        assert_eq!(
            stream.write(&[]).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            stream.flush().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            stream.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(control.writes.get(), writes);
        assert!(control.dropped.get());
        assert!(writer.is_faulted());
        assert!(writer.get_ref().is_empty());
    }
}

#[test]
fn finalization_buffer_failures_are_explicit_and_terminal() {
    for failure in 0..7 {
        let buffer = Buffer::new();
        let control = buffer.control.clone();
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut stream = writer
            .open_attribute_with_buffer(Tag::BODY, buffer)
            .unwrap();
        stream.write_all(b"body").unwrap();
        match failure {
            0 => control.fail_flush.set(true),
            1 => control.seek_failure.set(Some(control.seeks.get())),
            2 => control.seek_failure.set(Some(control.seeks.get() + 1)),
            3 => control.fail_read.set(true),
            4 => control.early_eof.set(true),
            5 => control.reported_length.set(Some(3)),
            6 => control.reported_length.set(Some(5)),
            _ => unreachable!(),
        }
        let expected = match failure {
            4 => io::ErrorKind::UnexpectedEof,
            5 | 6 => io::ErrorKind::InvalidData,
            _ => io::ErrorKind::Other,
        };
        assert_eq!(stream.finish().unwrap_err().kind(), expected);
        assert!(control.dropped.get());
        assert!(writer.is_faulted());
        let before = writer.get_ref().clone();
        if matches!(failure, 3 | 4) {
            assert_eq!(before.len(), 49, "no checksum after failed buffer read");
        } else {
            assert!(before.is_empty(), "invalid buffer emitted output");
        }
        assert_eq!(
            writer.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(writer.get_ref(), &before);
    }
}

#[test]
fn parent_sink_failures_during_child_finish_cannot_be_retried() {
    let mut expected = header(1252, 0);
    attribute(&mut expected, 1, Tag::BODY, b"body");
    for boundary in 0..expected.len() {
        let mut sink = super::Sink::new(7);
        sink.fail_after = Some(boundary);
        let calls = sink.calls.clone();
        let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
        let buffer = Buffer::new();
        let control = buffer.control.clone();
        let mut stream = writer
            .open_attribute_with_buffer(Tag::BODY, buffer)
            .unwrap();
        stream.write_all(b"body").unwrap();
        assert_eq!(stream.finish().unwrap_err().kind(), io::ErrorKind::Other);
        assert!(control.dropped.get());
        assert!(writer.is_faulted());
        assert_eq!(writer.get_ref().bytes, expected[..boundary]);
        let count = calls.get();
        super::assert_faulted(&mut writer);
        assert_eq!(calls.get(), count);
    }
}

#[test]
fn buffered_property_boundaries_and_attachments_follow_the_same_grammar() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_attribute(Tag::MAPI_PROPERTIES).unwrap();
    properties.write_all(&[0; 4]).unwrap();
    properties.finish().unwrap();
    assert!(writer.open_attribute(Tag::SUBJECT).is_err());
    assert!(writer.open_attribute(Tag::MAPI_PROPERTIES).is_err());
    for _ in 0..2 {
        let mut render = writer.open_attribute(Tag::ATTACH_RENDER_DATA).unwrap();
        render.write_all(&[0; 14]).unwrap();
        render.finish().unwrap();
        writer.write_attribute(Tag::ATTACH_DATA, b"data").unwrap();
        let mut properties = writer.open_attribute(Tag::ATTACHMENT).unwrap();
        properties.write_all(&[0; 4]).unwrap();
        properties.finish().unwrap();
        assert!(writer.open_attribute(Tag::ATTACH_DATA).is_err());
        assert!(writer.open_attribute(Tag::ATTACHMENT).is_err());
    }
    writer.finish().unwrap();
    assert_eq!(decode(writer.get_ref()).len(), 9);
}

#[test]
fn file_backed_attributes_and_property_values_match_memory_buffers_and_borrowed_files_survive() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let data = payload(100_000);
    let mut expected = header(1252, 0);
    attribute(&mut expected, 1, Tag::BODY, &data);
    let path = loop {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("tnef-attribute-{}-{id}.tmp", std::process::id()));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
                {
                    let mut stream = writer
                        .open_attribute_with_buffer(Tag::BODY, &mut file)
                        .unwrap();
                    for bytes in data.chunks(4096) {
                        stream.write_all(bytes).unwrap();
                    }
                    stream.finish().unwrap();
                }
                assert_eq!(file.stream_position().unwrap(), data.len() as u64);
                assert_eq!(file.metadata().unwrap().len(), data.len() as u64);
                writer.finish().unwrap();
                assert_eq!(writer.into_inner(), expected);
                file.set_len(0).unwrap();
                file.rewind().unwrap();
                let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
                let mut properties = writer
                    .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut file)
                    .unwrap();
                properties
                    .write_tag(mimecraft::tnef::TnefPropertyTag::ATTACH_DATA_BIN)
                    .unwrap();
                let mut value = properties.open_value().unwrap();
                for input in data.chunks(4096) {
                    value.write_all(input).unwrap();
                }
                value.finish().unwrap();
                properties.finish().unwrap();
                assert_eq!(
                    file.stream_position().unwrap(),
                    file.metadata().unwrap().len()
                );
                writer.finish().unwrap();
                let sets = super::properties::read_sets(writer.get_ref(), Tag::MAPI_PROPERTIES);
                assert_eq!(
                    sets[0][0].value(),
                    Some(&mimecraft::tnef::TnefValue::Binary(data.clone()))
                );
                for mode in [
                    mimecraft::tnef::RtfCompressionMode::Compressed,
                    mimecraft::tnef::RtfCompressionMode::Uncompressed,
                ] {
                    file.set_len(0).unwrap();
                    file.rewind().unwrap();
                    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
                    let mut properties = writer
                        .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut file)
                        .unwrap();
                    properties
                        .write_tag(mimecraft::tnef::TnefPropertyTag::RTF_COMPRESSED)
                        .unwrap();
                    let mut rtf = properties.open_rtf_compressed(mode).unwrap();
                    for input in data.chunks(4096) {
                        rtf.write_all(input).unwrap();
                    }
                    rtf.finish().unwrap();
                    properties.finish().unwrap();
                    assert_eq!(
                        file.stream_position().unwrap(),
                        file.metadata().unwrap().len()
                    );
                    writer.finish().unwrap();
                    let sets = super::properties::read_sets(writer.get_ref(), Tag::MAPI_PROPERTIES);
                    let Some(mimecraft::tnef::TnefValue::Binary(bytes)) = sets[0][0].value() else {
                        panic!("not binary");
                    };
                    assert_eq!(super::rtf::decoded(bytes, mode), data);
                }
                file.set_len(0).unwrap();
                file.rewind().unwrap();
                let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
                let mut properties = writer.open_properties(Tag::MAPI_PROPERTIES).unwrap();
                properties
                    .write_tag(mimecraft::tnef::TnefPropertyTag::ATTACH_DATA_OBJ)
                    .unwrap();
                let mut embedded = properties.open_embedded_message().unwrap();
                let mut inner = embedded
                    .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut file)
                    .unwrap();
                inner
                    .write_tag(mimecraft::tnef::TnefPropertyTag::ATTACH_DATA_BIN)
                    .unwrap();
                let mut value = inner.open_value().unwrap();
                value.write_all(&data).unwrap();
                value.finish().unwrap();
                inner.finish().unwrap();
                assert_eq!(
                    file.stream_position().unwrap(),
                    file.metadata().unwrap().len()
                );
                embedded.finish().unwrap();
                properties.finish().unwrap();
                writer.finish().unwrap();
                let sets = super::properties::read_sets(writer.get_ref(), Tag::MAPI_PROPERTIES);
                let Some(mimecraft::tnef::TnefValue::Object(bytes)) = sets[0][0].value() else {
                    panic!("not Object");
                };
                let sets = super::properties::read_sets(&bytes[16..], Tag::MAPI_PROPERTIES);
                assert_eq!(
                    sets[0][0].value(),
                    Some(&mimecraft::tnef::TnefValue::Binary(data.clone()))
                );
                drop(file);
                break path;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("could not create TNEF scratch file: {error}"),
        }
    };
    fs::remove_file(path).unwrap();
}

#[test]
fn borrowed_buffers_are_retained_on_failure() {
    let mut buffer = Buffer::new();
    let control = buffer.control.clone();
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut stream = writer
        .open_attribute_with_buffer(Tag::BODY, &mut buffer)
        .unwrap();
    stream.write_all(b"recoverable").unwrap();
    control.fail_flush.set(true);
    assert_eq!(stream.finish().unwrap_err().kind(), io::ErrorKind::Other);
    assert!(!control.dropped.get());
    assert_eq!(buffer.inner.get_ref(), b"recoverable");
    assert!(writer.is_faulted());
}

#[test]
fn every_partial_rtf_scratch_write_is_terminal_in_both_modes() {
    use mimecraft::tnef::{RtfCompressionMode as Mode, TnefPropertyTag as P, TnefValue as V};
    fn emit(writer: &mut TnefWriter<Vec<u8>>, buffer: &mut Buffer, mode: Mode) -> io::Result<()> {
        let mut properties = writer.open_properties_with_buffer(Tag::MAPI_PROPERTIES, buffer)?;
        properties.write_tag(P::RTF_COMPRESSED)?;
        let mut rtf = properties.open_rtf_compressed(mode)?;
        rtf.write_all(b"abc")?;
        rtf.finish()?;
        properties.finish()?;
        writer.finish()
    }
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        let mut baseline = Buffer::new();
        baseline.chunk = 1;
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        emit(&mut writer, &mut baseline, mode).unwrap();
        let sets = super::properties::read_sets(writer.get_ref(), Tag::MAPI_PROPERTIES);
        let Some(V::Binary(bytes)) = sets[0][0].value() else {
            panic!("not binary")
        };
        assert_eq!(super::rtf::decoded(bytes, mode), b"abc");
        let writes = baseline.control.writes.get();
        assert!(
            writes >= 55,
            "must include both headers, content and patches"
        );
        for boundary in 0..writes {
            let mut buffer = Buffer::new();
            buffer.chunk = 1;
            buffer.control.write_failure.set(Some(boundary));
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            assert_eq!(
                emit(&mut writer, &mut buffer, mode).unwrap_err().kind(),
                io::ErrorKind::Other,
                "{mode:?} {boundary}"
            );
            assert!(!buffer.control.dropped.get());
            assert!(writer.get_ref().is_empty());
            super::assert_faulted(&mut writer);
        }
    }
}

#[test]
fn rtf_header_value_patch_and_flush_failures_leave_properties_blocked() {
    use mimecraft::tnef::{RtfCompressionMode as Mode, TnefPropertyTag as P};
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        for failure in 0..8 {
            let mut buffer = Buffer::new();
            let control = buffer.control.clone();
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            let mut properties = writer
                .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut buffer)
                .unwrap();
            properties.write_tag(P::RTF_COMPRESSED).unwrap();
            let mut rtf = properties.open_rtf_compressed(mode).unwrap();
            rtf.write_all(b"abc").unwrap();
            if failure < 4 {
                control
                    .seek_failure
                    .set(Some(control.seeks.get() + failure));
            } else if failure == 4 {
                control.fail_flush.set(true);
            } else {
                // LZFu writes its final token group before either header patch.
                let group = usize::from(mode == Mode::Compressed);
                control
                    .write_failure
                    .set(Some(control.writes.get() + group + failure - 5));
                control.zero_write.set(true);
            }
            assert_eq!(
                rtf.finish().unwrap_err().kind(),
                if failure >= 5 {
                    io::ErrorKind::WriteZero
                } else {
                    io::ErrorKind::Other
                },
                "{mode:?} {failure}"
            );
            assert!(properties.is_faulted());
            let calls = (
                control.seeks.get(),
                control.writes.get(),
                control.flushes.get(),
            );
            assert_eq!(
                properties.open_rtf_compressed(mode).unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                properties.flush().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                properties.finish().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                (
                    control.seeks.get(),
                    control.writes.get(),
                    control.flushes.get()
                ),
                calls
            );
            assert!(!control.dropped.get());
            assert!(writer.get_ref().is_empty());
            super::assert_faulted(&mut writer);
        }
    }
}

#[test]
fn rtf_encoding_write_failures_preserve_acknowledged_raw_size_and_are_terminal() {
    use mimecraft::tnef::{RtfCompressionMode as Mode, TnefPropertyTag as P};
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        for zero in [false, true] {
            let buffer = Buffer::new();
            let control = buffer.control.clone();
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            let mut properties = writer
                .open_properties_with_buffer(Tag::MAPI_PROPERTIES, buffer)
                .unwrap();
            properties.write_tag(P::RTF_COMPRESSED).unwrap();
            let mut rtf = properties.open_rtf_compressed(mode).unwrap();
            rtf.write_all(&[b'x'; 4096]).unwrap();
            assert_eq!(rtf.raw_size(), 4096);
            control.fail_write.set(true);
            control.zero_write.set(zero);
            assert_eq!(
                rtf.write(b"x").unwrap_err().kind(),
                if zero {
                    io::ErrorKind::WriteZero
                } else {
                    io::ErrorKind::Other
                }
            );
            assert_eq!(rtf.raw_size(), 4096);
            assert!(rtf.is_faulted());
            let calls = control.writes.get();
            assert_eq!(
                rtf.write(&[]).unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(rtf.flush().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(rtf.finish().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(control.writes.get(), calls);
            assert!(properties.is_faulted());
            assert!(!control.dropped.get());
            assert_eq!(
                properties.finish().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert!(control.dropped.get());
            super::assert_faulted(&mut writer);
        }
    }
}

#[test]
fn rtf_short_interrupted_io_and_drop_never_emit_unfinished_attributes() {
    use mimecraft::tnef::{RtfCompressionMode as Mode, TnefPropertyTag as P, TnefValue as V};
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        for abandon in [false, true] {
            let mut buffer = Buffer::new();
            buffer.chunk = 1;
            let control = buffer.control.clone();
            let sink = super::Sink::new(7);
            let calls = sink.calls.clone();
            let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
            let mut properties = writer
                .open_properties_with_buffer(Tag::MAPI_PROPERTIES, buffer)
                .unwrap();
            properties.write_tag(P::RTF_COMPRESSED).unwrap();
            let snapshot;
            {
                control.interrupt_write.set(true);
                let mut rtf = properties.open_rtf_compressed(mode).unwrap();
                control.interrupt_write.set(true);
                rtf.write_all(b"{\\rtf1 test}").unwrap();
                control.interrupt_flush.set(true);
                rtf.flush().unwrap();
                assert_eq!(calls.get(), 0);
                if !abandon {
                    control.interrupt_seek.set(true);
                    control.interrupt_write.set(true);
                    control.interrupt_flush.set(true);
                    rtf.finish().unwrap();
                }
                snapshot = (
                    control.seeks.get(),
                    control.writes.get(),
                    control.flushes.get(),
                );
            }
            assert_eq!(
                (
                    control.seeks.get(),
                    control.writes.get(),
                    control.flushes.get()
                ),
                snapshot
            );
            assert!(!control.dropped.get());
            assert_eq!(calls.get(), 0);
            if abandon {
                assert_eq!(
                    properties.finish().unwrap_err().kind(),
                    io::ErrorKind::BrokenPipe
                );
                assert!(writer.get_ref().bytes.is_empty());
                super::assert_faulted(&mut writer);
            } else {
                control.interrupt_read.set(true);
                properties.finish().unwrap();
                writer.finish().unwrap();
                let sets =
                    super::properties::read_sets(&writer.get_ref().bytes, Tag::MAPI_PROPERTIES);
                let Some(V::Binary(bytes)) = sets[0][0].value() else {
                    panic!("not binary")
                };
                assert_eq!(super::rtf::decoded(bytes, mode), b"{\\rtf1 test}");
            }
            assert!(control.dropped.get());
        }
    }
}

#[test]
fn every_partial_embedded_scratch_write_is_terminal() {
    use mimecraft::tnef::TnefPropertyTag as P;
    fn emit(writer: &mut TnefWriter<Vec<u8>>, buffer: &mut Buffer) -> io::Result<()> {
        let mut properties = writer.open_properties_with_buffer(Tag::MAPI_PROPERTIES, buffer)?;
        properties.write_tag(P::ATTACH_DATA_OBJ)?;
        let mut embedded = properties.open_embedded_message()?;
        embedded.write_string(Tag::SUBJECT, "inner")?;
        let mut inner = embedded.open_properties(Tag::MAPI_PROPERTIES)?;
        inner.write_tag(P::ATTACH_DATA_BIN)?;
        let mut value = inner.open_value()?;
        value.write_all(b"payload")?;
        value.finish()?;
        inner.finish()?;
        embedded.finish()?;
        properties.finish()?;
        writer.finish()
    }
    let mut baseline = Buffer::new();
    baseline.chunk = 1;
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    emit(&mut writer, &mut baseline).unwrap();
    let writes = baseline.control.writes.get();
    assert!(
        writes > 100,
        "must include IID, nested attributes and patches"
    );
    for zero in [false, true] {
        for boundary in 0..writes {
            let mut buffer = Buffer::new();
            buffer.chunk = 1;
            buffer.control.write_failure.set(Some(boundary));
            buffer.control.zero_write.set(zero);
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            assert_eq!(
                emit(&mut writer, &mut buffer).unwrap_err().kind(),
                if zero {
                    io::ErrorKind::WriteZero
                } else {
                    io::ErrorKind::Other
                },
                "{zero} {boundary}"
            );
            assert!(!buffer.control.dropped.get());
            assert!(writer.get_ref().is_empty());
            super::assert_faulted(&mut writer);
        }
    }
}

#[test]
fn embedded_finish_flush_length_patch_and_padding_errors_block_parents() {
    use mimecraft::tnef::TnefPropertyTag as P;
    for failure in 0..7 {
        let mut buffer = Buffer::new();
        let control = buffer.control.clone();
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer
            .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut buffer)
            .unwrap();
        properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
        let mut embedded = properties.open_embedded_message().unwrap();
        embedded.write_string(Tag::SUBJECT, "inner").unwrap();
        match failure {
            0 => control.fail_flush.set(true),
            1 | 2 => control
                .seek_failure
                .set(Some(control.seeks.get() + failure - 1)),
            3..=6 => {
                control
                    .write_failure
                    .set(Some(control.writes.get() + (failure - 3) % 2));
                control.zero_write.set(failure >= 5);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            embedded.finish().unwrap_err().kind(),
            if failure >= 5 {
                io::ErrorKind::WriteZero
            } else {
                io::ErrorKind::Other
            }
        );
        assert!(properties.is_faulted());
        assert_eq!(
            properties.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(!control.dropped.get());
        assert!(writer.get_ref().is_empty());
        super::assert_faulted(&mut writer);
    }
}

#[test]
fn embedded_nested_scratch_failure_and_abandonment_do_not_emit_outer_attributes() {
    use mimecraft::tnef::TnefPropertyTag as P;
    for fail in [false, true] {
        let mut buffer = Buffer::new();
        let control = buffer.control.clone();
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer
            .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut buffer)
            .unwrap();
        properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
        {
            let mut embedded = properties.open_embedded_message().unwrap();
            let mut scratch = Buffer::new();
            let nested_control = scratch.control.clone();
            let mut inner = embedded
                .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut scratch)
                .unwrap();
            inner.write_tag(P::SUBJECT_W).unwrap();
            inner
                .write_value(&mimecraft::tnef::TnefValue::Text("inner".into()))
                .unwrap();
            nested_control.fail_flush.set(fail);
            if fail {
                assert_eq!(inner.finish().unwrap_err().kind(), io::ErrorKind::Other);
                assert!(embedded.is_faulted());
                assert_eq!(
                    embedded.finish().unwrap_err().kind(),
                    io::ErrorKind::BrokenPipe
                );
            } else {
                drop(inner);
                let before = (
                    control.writes.get(),
                    control.flushes.get(),
                    nested_control.writes.get(),
                    nested_control.flushes.get(),
                );
                drop(embedded);
                assert_eq!(
                    (
                        control.writes.get(),
                        control.flushes.get(),
                        nested_control.writes.get(),
                        nested_control.flushes.get()
                    ),
                    before
                );
            }
            assert!(!nested_control.dropped.get());
        }
        assert!(properties.is_faulted());
        assert_eq!(
            properties.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(!control.dropped.get());
        assert!(writer.get_ref().is_empty());
        super::assert_faulted(&mut writer);
    }
}

#[test]
fn embedded_short_interrupted_nested_io_preserves_complete_content() {
    use mimecraft::tnef::{TnefPropertyTag as P, TnefValue as V};
    let mut outer = Buffer::new();
    outer.chunk = 1;
    let mut inner = Buffer::new();
    inner.chunk = 7;
    for control in [&outer.control, &inner.control] {
        control.interrupt_seek.set(true);
        control.interrupt_write.set(true);
        control.interrupt_read.set(true);
        control.interrupt_flush.set(true);
    }
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer
        .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut outer)
        .unwrap();
    properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let mut embedded = properties.open_embedded_message().unwrap();
    let mut nested = embedded
        .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut inner)
        .unwrap();
    nested.write_tag(P::SUBJECT_W).unwrap();
    nested.write_value(&V::Text("inner".into())).unwrap();
    nested.finish().unwrap();
    embedded.flush().unwrap();
    embedded.finish().unwrap();
    properties.finish().unwrap();
    writer.finish().unwrap();
    let sets = super::properties::read_sets(writer.get_ref(), Tag::MAPI_PROPERTIES);
    let Some(V::Object(bytes)) = sets[0][0].value() else {
        panic!("not Object")
    };
    let sets = super::properties::read_sets(&bytes[16..], Tag::MAPI_PROPERTIES);
    assert_eq!(sets[0][0].value(), Some(&V::Text("inner".into())));
    assert!(!outer.control.dropped.get());
    assert!(!inner.control.dropped.get());
}

#[test]
fn embedded_attributes_use_independently_owned_scratch_buffers() {
    use mimecraft::tnef::{TnefPropertyTag as P, TnefValue as V};
    // Rust supplies scratch per attribute rather than inheriting a stream factory.
    let outer = Buffer::new();
    let outer_control = outer.control.clone();
    let inner = Buffer::new();
    let inner_control = inner.control.clone();
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer
        .open_properties_with_buffer(Tag::MAPI_PROPERTIES, outer)
        .unwrap();
    properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let mut embedded = properties.open_embedded_message().unwrap();
    let mut nested = embedded
        .open_properties_with_buffer(Tag::MAPI_PROPERTIES, inner)
        .unwrap();
    nested.write_tag(P::SUBJECT_W).unwrap();
    nested.write_value(&V::Text("inner".into())).unwrap();
    drop(nested.finish().unwrap());
    assert!(inner_control.dropped.get());
    assert!(!outer_control.dropped.get());
    embedded.finish().unwrap();
    drop(properties.finish().unwrap());
    assert!(outer_control.dropped.get());
    writer.finish().unwrap();
    let sets = super::properties::read_sets(writer.get_ref(), Tag::MAPI_PROPERTIES);
    let Some(V::Object(bytes)) = sets[0][0].value() else {
        panic!("not Object")
    };
    let sets = super::properties::read_sets(&bytes[16..], Tag::MAPI_PROPERTIES);
    assert_eq!(sets[0][0].value(), Some(&V::Text("inner".into())));
}

#[derive(Debug, Default)]
struct Control {
    reads: Cell<usize>,
    writes: Cell<usize>,
    seeks: Cell<usize>,
    flushes: Cell<usize>,
    dropped: Cell<bool>,
    fail_write: Cell<bool>,
    write_failure: Cell<Option<usize>>,
    zero_write: Cell<bool>,
    fail_read: Cell<bool>,
    early_eof: Cell<bool>,
    fail_flush: Cell<bool>,
    seek_failure: Cell<Option<usize>>,
    reported_length: Cell<Option<u64>>,
    interrupt_write: Cell<bool>,
    interrupt_read: Cell<bool>,
    interrupt_seek: Cell<bool>,
    interrupt_flush: Cell<bool>,
}

#[derive(Debug)]
struct Buffer {
    inner: Cursor<Vec<u8>>,
    control: Rc<Control>,
    chunk: usize,
}

impl Buffer {
    fn new() -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            control: Rc::new(Control::default()),
            chunk: usize::MAX,
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.control.dropped.set(true);
    }
}

impl Read for Buffer {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.control.reads.set(self.control.reads.get() + 1);
        if self.control.interrupt_read.replace(false) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.control.fail_read.get() {
            return Err(io::Error::other("buffer read failure"));
        }
        if self.control.early_eof.get() {
            return Ok(0);
        }
        let count = buffer.len().min(self.chunk);
        self.inner.read(&mut buffer[..count])
    }
}

impl Write for Buffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.control.writes.set(self.control.writes.get() + 1);
        if self.control.interrupt_write.replace(false) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.control.fail_write.get()
            || self.control.write_failure.get() == Some(self.control.writes.get() - 1)
        {
            return if self.control.zero_write.get() {
                Ok(0)
            } else {
                Err(io::Error::other("buffer write failure"))
            };
        }
        self.inner.write(&buffer[..buffer.len().min(self.chunk)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.control.flushes.set(self.control.flushes.get() + 1);
        if self.control.interrupt_flush.replace(false) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.control.fail_flush.get() {
            Err(io::Error::other("buffer flush failure"))
        } else {
            Ok(())
        }
    }
}

impl Seek for Buffer {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let count = self.control.seeks.get();
        self.control.seeks.set(count + 1);
        if self.control.interrupt_seek.replace(false) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.control.seek_failure.get() == Some(count) {
            return Err(io::Error::other("buffer seek failure"));
        }
        let actual = self.inner.seek(position)?;
        if position == SeekFrom::End(0) {
            Ok(self.control.reported_length.get().unwrap_or(actual))
        } else {
            Ok(actual)
        }
    }
}

#[test]
fn property_buffer_initial_write_and_invalid_opens() {
    for tag in [
        Tag::SUBJECT,
        Tag::ATTACH_DATA,
        Tag::TNEF_VERSION,
        Tag::ATTACHMENT,
    ] {
        let buffer = Buffer::new();
        let control = buffer.control.clone();
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        assert!(writer.open_properties_with_buffer(tag, buffer).is_err());
        assert_eq!(control.seeks.get(), 0);
        assert_eq!(control.writes.get(), 0);
        assert!(control.dropped.get());
        assert!(!writer.is_faulted());
        writer.finish().unwrap();
    }

    let buffer = Buffer::new();
    let control = buffer.control.clone();
    control.fail_write.set(true);
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    assert_eq!(
        writer
            .open_properties_with_buffer(Tag::MAPI_PROPERTIES, buffer)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Other,
    );
    assert!(writer.is_faulted());
    assert!(writer.get_ref().is_empty());
    assert!(control.dropped.get());
}

#[test]
fn every_partial_streamed_property_write_including_length_and_padding_is_terminal() {
    use mimecraft::tnef::{TnefPropertyTag as P, TnefPropertyType as Type};
    for multi in [false, true] {
        // Count/tag/value count/length, five bytes, length patch, padding, and
        // property count patch; multivalues also patch their value count.
        let writes = if multi { 36 } else { 32 };
        for boundary in 0..writes {
            let mut buffer = Buffer::new();
            buffer.chunk = 1;
            buffer.control.write_failure.set(Some(boundary));
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            let result = (|| {
                let mut properties =
                    writer.open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut buffer)?;
                properties.write_tag(P::new(
                    0x6000,
                    Type(Type::BINARY.0 | if multi { Type::MULTI_VALUED.0 } else { 0 }),
                ))?;
                let mut value = properties.open_value()?;
                value.write_all(&[1, 2, 3, 4, 5])?;
                value.finish()?;
                properties.finish().map(|_| ())
            })();
            assert_eq!(
                result.unwrap_err().kind(),
                io::ErrorKind::Other,
                "{multi}, {boundary}"
            );
            assert!(!buffer.control.dropped.get());
            assert!(writer.get_ref().is_empty());
            super::assert_faulted(&mut writer);
        }
    }
}

#[test]
fn streamed_property_open_write_flush_and_finish_failures_block_recovery() {
    use mimecraft::tnef::TnefPropertyTag as P;
    for failure in 0..10 {
        let mut buffer = Buffer::new();
        let control = buffer.control.clone();
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer
            .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut buffer)
            .unwrap();
        properties.write_tag(P::ATTACH_DATA_BIN).unwrap();
        let error = if failure == 0 {
            control.fail_write.set(true);
            properties.open_value().unwrap_err()
        } else {
            let mut value = properties.open_value().unwrap();
            value.write_all(&[1, 2, 3, 4, 5]).unwrap();
            match failure {
                1 | 4 | 7 | 8 => control.fail_write.set(true),
                2 => control.fail_flush.set(true),
                3 => control.seek_failure.set(Some(control.seeks.get())),
                5 => control.seek_failure.set(Some(control.seeks.get() + 1)),
                6 | 9 => control.write_failure.set(Some(control.writes.get() + 1)),
                _ => unreachable!(),
            }
            control.zero_write.set(failure >= 7);
            if matches!(failure, 1 | 2 | 7) {
                let error = if failure == 2 {
                    value.flush()
                } else {
                    value.write_all(&[6])
                }
                .unwrap_err();
                assert!(value.is_faulted());
                assert_eq!(value.len(), 5);
                let calls = (
                    control.writes.get(),
                    control.seeks.get(),
                    control.flushes.get(),
                );
                assert_eq!(
                    value.write(&[]).unwrap_err().kind(),
                    io::ErrorKind::BrokenPipe
                );
                assert_eq!(
                    value.write(&[7]).unwrap_err().kind(),
                    io::ErrorKind::BrokenPipe
                );
                assert_eq!(value.flush().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
                assert_eq!(
                    value.finish().unwrap_err().kind(),
                    io::ErrorKind::BrokenPipe
                );
                assert_eq!(
                    (
                        control.writes.get(),
                        control.seeks.get(),
                        control.flushes.get()
                    ),
                    calls
                );
                error
            } else {
                value.finish().unwrap_err()
            }
        };
        assert_eq!(
            error.kind(),
            if failure >= 7 {
                io::ErrorKind::WriteZero
            } else {
                io::ErrorKind::Other
            }
        );
        assert!(properties.is_faulted());
        let calls = (
            control.writes.get(),
            control.seeks.get(),
            control.flushes.get(),
        );
        assert_eq!(
            properties.open_value().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            properties.flush().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            properties.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            (
                control.writes.get(),
                control.seeks.get(),
                control.flushes.get()
            ),
            calls
        );
        assert!(!control.dropped.get());
        assert!(writer.get_ref().is_empty());
        super::assert_faulted(&mut writer);
    }
}

#[test]
fn streamed_values_handle_short_interrupted_io_and_flush_only_scratch() {
    use mimecraft::tnef::{TnefPropertyTag as P, TnefPropertyType as Type, TnefValue as V};
    for chunk in [1, 7, 4096] {
        let mut buffer = Buffer::new();
        buffer.chunk = chunk;
        let control = buffer.control.clone();
        let sink = super::Sink::new(7);
        let calls = sink.calls.clone();
        let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
        let mut properties = writer
            .open_properties_with_buffer(Tag::MAPI_PROPERTIES, buffer)
            .unwrap();
        properties
            .write_tag(P::new(0x6000, Type(Type::BINARY.0 | Type::MULTI_VALUED.0)))
            .unwrap();
        let mut expected = Vec::new();
        for length in [0, 5, 2049] {
            let data = payload(length);
            control.interrupt_write.set(true);
            let mut value = properties.open_value().unwrap();
            control.interrupt_write.set(true);
            value.write_all(&data).unwrap();
            control.interrupt_flush.set(true);
            value.flush().unwrap();
            let flushes = control.flushes.get();
            control.interrupt_seek.set(true);
            control.interrupt_write.set(true);
            value.finish().unwrap();
            assert_eq!(
                control.flushes.get(),
                flushes,
                "value finish must not flush"
            );
            assert_eq!(calls.get(), 0, "value completion must not emit output");
            expected.push(V::Binary(data));
        }
        control.interrupt_read.set(true);
        control.interrupt_flush.set(true);
        properties.finish().unwrap();
        writer.finish().unwrap();
        let sets = super::properties::read_sets(&writer.get_ref().bytes, Tag::MAPI_PROPERTIES);
        assert_eq!(sets[0][0].values(), expected);
    }
}

#[test]
fn dropping_a_value_neither_performs_io_nor_drops_attribute_storage() {
    use mimecraft::tnef::TnefPropertyTag as P;
    let buffer = Buffer::new();
    let control = buffer.control.clone();
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer
        .open_properties_with_buffer(Tag::MAPI_PROPERTIES, buffer)
        .unwrap();
    properties.write_tag(P::ATTACH_DATA_BIN).unwrap();
    let calls = {
        let mut value = properties.open_value().unwrap();
        value.write_all(&[1, 2, 3]).unwrap();
        (
            control.writes.get(),
            control.seeks.get(),
            control.flushes.get(),
        )
    };
    assert_eq!(
        (
            control.writes.get(),
            control.seeks.get(),
            control.flushes.get()
        ),
        calls
    );
    assert!(!control.dropped.get());
    assert!(properties.is_faulted());
    let buffer = properties.into_inner();
    assert_eq!(&buffer.inner.get_ref()[12..], &[0, 0, 0, 0, 1, 2, 3]);
    drop(buffer);
    assert!(control.dropped.get());
    assert!(writer.get_ref().is_empty());
    super::assert_faulted(&mut writer);
}

#[test]
fn every_partial_property_scratch_write_faults_parent() {
    use mimecraft::tnef::{TnefPropertyTag as P, TnefValue as V};
    // Four-byte count, four-byte tag, four-byte value, then a four-byte count patch.
    for boundary in 0..16 {
        let mut buffer = Buffer::new();
        buffer.chunk = 1;
        buffer.control.write_failure.set(Some(boundary));
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let result = (|| {
            let mut properties =
                writer.open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut buffer)?;
            properties.write_tag(P::IMPORTANCE)?;
            properties.write_value(&V::I32(7))?;
            properties.finish().map(|_| ())
        })();
        assert_eq!(
            result.unwrap_err().kind(),
            io::ErrorKind::Other,
            "{boundary}"
        );
        assert!(!buffer.control.dropped.get());
        assert!(writer.is_faulted());
        assert!(writer.get_ref().is_empty());
        assert_eq!(
            writer.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}

#[test]
fn property_count_patching_failures_are_terminal() {
    use mimecraft::tnef::{TnefPropertyTag as P, TnefPropertyType as Type, TnefValue as V};
    for failure in 0..5 {
        let buffer = Buffer::new();
        let control = buffer.control.clone();
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer
            .open_properties_with_buffer(Tag::MAPI_PROPERTIES, buffer)
            .unwrap();
        properties
            .write_tag(P::new(0x6000, Type(Type::LONG.0 | Type::MULTI_VALUED.0)))
            .unwrap();
        properties.write_value(&V::I32(1)).unwrap();
        match failure {
            0 => control.seek_failure.set(Some(control.seeks.get())),
            1 => control.fail_write.set(true),
            2 => control.seek_failure.set(Some(control.seeks.get() + 1)),
            3 => {
                control.fail_write.set(true);
                control.zero_write.set(true);
            }
            _ => control.fail_flush.set(true),
        }
        let error = if failure == 4 {
            properties.flush()
        } else {
            properties.write_tag(P::SUBJECT_W)
        }
        .unwrap_err();
        assert_eq!(
            error.kind(),
            if failure == 3 {
                io::ErrorKind::WriteZero
            } else {
                io::ErrorKind::Other
            }
        );
        assert!(properties.is_faulted());
        let calls = (
            control.seeks.get(),
            control.writes.get(),
            control.flushes.get(),
        );
        assert_eq!(
            properties.write_tag(P::SUBJECT_W).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            properties.write_value(&V::I32(2)).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            properties.begin_row().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            properties.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            (
                control.seeks.get(),
                control.writes.get(),
                control.flushes.get()
            ),
            calls
        );
        assert!(writer.is_faulted());
        assert!(writer.get_ref().is_empty());
        assert!(control.dropped.get());
    }
}

#[test]
fn property_short_and_interrupted_patches_rows_and_replay_round_trip() {
    use mimecraft::tnef::{
        TnefPropertyTag as P, TnefPropertyType as Type, TnefReader, TnefValue as V,
    };
    for chunk in [1, 7, 4096] {
        let mut buffer = Buffer::new();
        buffer.chunk = chunk;
        let control = buffer.control.clone();
        let mut writer = TnefWriter::new(super::Sink::new(7), 1252, 0).unwrap();
        let mut properties = writer
            .open_properties_with_buffer(Tag::RECIPIENT_TABLE, buffer)
            .unwrap();
        for _ in 0..3 {
            control.interrupt_seek.set(true);
            control.interrupt_write.set(true);
            properties.begin_row().unwrap();
            properties
                .write_tag(P::new(0x6000, Type(Type::LONG.0 | Type::MULTI_VALUED.0)))
                .unwrap();
            for number in 0..5 {
                properties.write_value(&V::I32(number)).unwrap();
            }
        }
        control.interrupt_seek.set(true);
        control.interrupt_write.set(true);
        control.interrupt_read.set(true);
        control.interrupt_flush.set(true);
        properties.finish().unwrap();
        writer.finish().unwrap();
        let bytes = writer.into_inner().bytes;
        let mut reader = TnefReader::new(bytes.as_slice());
        for _ in 0..3 {
            assert!(reader.read_next().unwrap());
        }
        let mut props = reader.open_properties().unwrap();
        for _ in 0..3 {
            assert!(props.read_next_row().unwrap());
            assert!(props.read_next_property().unwrap());
            let property = props.read_property().unwrap();
            assert_eq!(property.values(), (0..5).map(V::I32).collect::<Vec<_>>());
            assert!(!props.read_next_property().unwrap());
        }
        assert!(!props.read_next_row().unwrap());
        drop(props);
        assert!(!reader.read_next().unwrap());
    }
}

#[test]
fn property_finish_sink_failures_leave_borrowed_scratch_and_parent_faulted() {
    use mimecraft::tnef::{TnefPropertyTag as P, TnefValue as V};
    let mut expected = header(1252, 0);
    let payload = [1, 0, 0, 0, 3, 0, 0x17, 0, 7, 0, 0, 0];
    attribute(&mut expected, 1, Tag::MAPI_PROPERTIES, &payload);
    for boundary in 0..expected.len() {
        let mut sink = super::Sink::new(7);
        sink.fail_after = Some(boundary);
        let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
        let mut buffer = Buffer::new();
        let mut properties = writer
            .open_properties_with_buffer(Tag::MAPI_PROPERTIES, &mut buffer)
            .unwrap();
        properties.write_tag(P::IMPORTANCE).unwrap();
        properties.write_value(&V::I32(7)).unwrap();
        assert_eq!(
            properties.finish().unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(buffer.inner.get_ref(), &payload);
        assert!(!buffer.control.dropped.get());
        assert_eq!(writer.get_ref().bytes, expected[..boundary]);
        super::assert_faulted(&mut writer);
    }
}

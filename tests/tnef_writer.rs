// Adapted from MimeKit's UnitTests/Tnef/TnefWriterTests.cs (attribute-writing tests).
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::tnef::{
    TnefAttributeLevel as Level, TnefAttributeTag as Tag, TnefDateTime, TnefDateTimeKind as Kind,
    TnefReader, TnefWriter,
};
use std::{
    cell::Cell,
    io::{self, Read, Write},
    rc::Rc,
};

#[path = "tnef_writer/buffered.rs"]
mod buffered;

fn attribute(output: &mut Vec<u8>, level: u8, tag: Tag, value: &[u8]) {
    output.push(level);
    output.extend_from_slice(&tag.0.to_le_bytes());
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(value);
    let checksum = value
        .iter()
        .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)));
    output.extend_from_slice(&checksum.to_le_bytes());
}

fn header(codepage: u32, key: u16) -> Vec<u8> {
    let mut output = vec![0x78, 0x9f, 0x3e, 0x22];
    output.extend_from_slice(&key.to_le_bytes());
    attribute(&mut output, 1, Tag::TNEF_VERSION, &[0, 0, 1, 0]);
    let mut codepages = [0; 8];
    codepages[..4].copy_from_slice(&codepage.to_le_bytes());
    attribute(&mut output, 1, Tag::OEM_CODEPAGE, &codepages);
    output
}

fn decode(bytes: &[u8]) -> Vec<(Level, Tag, Vec<u8>)> {
    let mut reader = TnefReader::new(bytes);
    let mut values = Vec::new();
    let mut logger = |issue| panic!("unexpected TNEF compliance issue: {issue:?}");
    while reader.read_next_with_logger(&mut logger).unwrap() {
        let info = *reader.attribute().unwrap();
        let bytes = reader.read_value_bytes_with_logger(&mut logger).unwrap();
        values.push((info.level, info.tag, bytes));
    }
    assert!(!reader.is_truncated());
    values
}

fn date() -> TnefDateTime {
    TnefDateTime::new(2024, 1, 2, 3, 4, 5, Kind::Unspecified).unwrap()
}

#[test]
fn empty_stream_header_properties_and_explicit_finish() {
    for (codepage, key) in [(1252, 0), (1251, 0x1234), (65001, 0xbeef)] {
        let mut writer = TnefWriter::new(Vec::new(), codepage, key).unwrap();
        assert_eq!(writer.codepage(), codepage);
        assert_eq!(writer.legacy_key(), key);
        assert!(writer.get_ref().is_empty(), "construction performed I/O");
        assert!(!writer.is_finished());
        assert!(!writer.is_faulted());
        writer.finish().unwrap();
        let expected = header(codepage, key);
        assert_eq!(expected.len(), 40); // Signature, legacy key, version, and OEM codepage.
        assert_eq!(writer.get_ref(), &expected);
        writer.finish().unwrap();
        writer.flush().unwrap();
        assert_eq!(writer.get_ref(), &expected);
        assert!(writer.is_finished());
        assert_eq!(decode(&expected).len(), 2);
        let mut reader = TnefReader::new(expected.as_slice());
        while reader.read_next().unwrap() {}
        assert_eq!(reader.codepage(), codepage);
        assert_eq!(reader.legacy_key(), Some(key));
    }
}

#[test]
fn byte_exact_against_upstream_builder_vector() {
    let mut expected = header(1252, 0x0102);
    attribute(
        &mut expected,
        1,
        Tag::MESSAGE_CLASS,
        b"IPM.Microsoft Mail.Note\0",
    );
    attribute(&mut expected, 1, Tag::SUBJECT, b"Hello\0");
    attribute(&mut expected, 1, Tag::PRIORITY, &[2, 0]);
    attribute(
        &mut expected,
        1,
        Tag::DATE_SENT,
        &[0xe8, 7, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 2, 0],
    );
    attribute(&mut expected, 2, Tag::ATTACH_RENDER_DATA, &[0; 14]);
    attribute(&mut expected, 2, Tag::ATTACH_DATA, &[0xff, 0xfe, 0x80]);
    for chunk in [1, 7, 4096] {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0x0102).unwrap();
        writer
            .write_string(Tag::MESSAGE_CLASS, "IPM.Microsoft Mail.Note")
            .unwrap();
        writer.write_string(Tag::SUBJECT, "Hello").unwrap();
        writer.write_i32(Tag::PRIORITY, 2).unwrap();
        writer.write_datetime(Tag::DATE_SENT, date()).unwrap();
        writer
            .write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])
            .unwrap();
        let mut source = Source::new(&[0xff, 0xfe, 0x80], chunk);
        writer
            .write_attribute_from(Tag::ATTACH_DATA, &mut source, 3)
            .unwrap();
        writer.finish().unwrap();
        assert_eq!(writer.into_inner(), expected);
        assert_eq!(decode(&expected).len(), 8);
    }
}

#[test]
fn typed_values_and_codepages_round_trip() {
    let cases = [
        (1252, "Subject \u{e9}", b"Subject \xe9\0".as_slice()),
        (
            1251,
            "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}",
            b"\xcf\xf0\xe8\xe2\xe5\xf2\0",
        ),
        (65001, "Subject \u{1f600}", b"Subject \xf0\x9f\x98\x80\0"),
        (20127, "Hello", b"Hello\0"),
        (28591, "Subject \u{ff}", b"Subject \xff\0"),
        (932, "\u{65e5}\u{672c}", b"\x93\xfa\x96\x7b\0"),
    ];
    for (codepage, subject, encoded) in cases {
        let mut writer = TnefWriter::new(Vec::new(), codepage, 0).unwrap();
        writer.write_string(Tag::MESSAGE_CLASS, "IPM.Note").unwrap();
        writer.write_string(Tag::SUBJECT, subject).unwrap();
        writer.write_string(Tag::BODY, "Body text").unwrap();
        writer.write_i32(Tag::PRIORITY, 1).unwrap();
        writer.write_i32(Tag::AID_OWNER, -5).unwrap();
        writer.write_datetime(Tag::DATE_SENT, date()).unwrap();
        writer.finish().unwrap();
        let bytes = writer.into_inner();
        let values = decode(&bytes);
        assert_eq!(values[3].2, encoded);
        assert_eq!(values[4].2, b"Body text\0");
        assert_eq!(values[5].2, [1, 0]);
        assert_eq!(values[6].2, (-5i32).to_le_bytes());
        let mut reader = TnefReader::new(bytes.as_slice());
        while reader.read_next().unwrap() {
            match reader.attribute().unwrap().tag {
                Tag::SUBJECT => assert_eq!(reader.read_value_string().unwrap(), subject),
                Tag::DATE_SENT => assert_eq!(reader.read_value_datetime().unwrap(), date()),
                _ => {}
            }
        }
    }
}

#[test]
fn integer_limits_and_raw_subslices() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    for value in [i16::MIN as i32, 0, i16::MAX as i32] {
        writer.write_i32(Tag::PRIORITY, value).unwrap();
    }
    for value in [i32::MIN, -5, 0, i32::MAX] {
        writer.write_i32(Tag::AID_OWNER, value).unwrap();
    }
    writer
        .write_attribute(Tag::BODY, &b"xbody\0y"[1..6])
        .unwrap();
    writer
        .write_attribute(Tag::MESSAGE_STATUS, &[0x20])
        .unwrap();
    writer.finish().unwrap();
    let values = decode(writer.get_ref());
    for (index, value) in [i16::MIN, 0, i16::MAX].into_iter().enumerate() {
        assert_eq!(values[index + 2].2, value.to_le_bytes());
    }
    for (index, value) in [i32::MIN, -5, 0, i32::MAX].into_iter().enumerate() {
        assert_eq!(values[index + 5].2, value.to_le_bytes());
    }
    assert_eq!(values[9].2, b"body\0");
}

#[test]
fn dates_preserve_calendar_components_and_emit_sunday_zero() {
    for (year, month, day, weekday) in [
        (1, 1, 1, 1),
        (2023, 12, 31, 0),
        (2024, 2, 29, 4),
        (9999, 12, 31, 5),
    ] {
        for kind in [Kind::Unspecified, Kind::Utc] {
            let base = TnefDateTime::new(year, month, day, 23, 59, 58, kind).unwrap();
            let value = TnefDateTime::from_ticks(base.ticks() + 9_999_999, kind).unwrap();
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            writer.write_datetime(Tag::DATE_SENT, value).unwrap();
            writer.finish().unwrap();
            let values = decode(writer.get_ref());
            assert_eq!(&values[2].2[12..], &[weekday, 0]);
            let mut reader = TnefReader::new(writer.get_ref().as_slice());
            assert!(reader.read_next().unwrap());
            assert!(reader.read_next().unwrap());
            assert!(reader.read_next().unwrap());
            let actual = reader.read_value_datetime().unwrap();
            assert_eq!(actual.ticks(), base.ticks());
            assert_eq!(actual.kind(), Kind::Unspecified);
        }
    }
}

#[test]
fn invalid_codepages_do_not_touch_sink() {
    for codepage in [0, 1200, 1201, 12000, 12001, 99999, u32::MAX] {
        let sink = Sink::new(usize::MAX);
        let calls = sink.calls.clone();
        assert_eq!(
            TnefWriter::new(sink, codepage, 0).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(calls.get(), 0);
    }
}

#[test]
fn invalid_tags_types_and_values_preserve_state_and_output() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let invalid = io::ErrorKind::InvalidInput;
    for tag in [
        Tag::TNEF_VERSION,
        Tag::OEM_CODEPAGE,
        Tag::NULL,
        Tag(0x00061234),
    ] {
        assert_eq!(
            writer.write_attribute(tag, &[]).unwrap_err().kind(),
            invalid
        );
    }
    for tag in [
        Tag::SUBJECT,
        Tag::DATE_SENT,
        Tag::MESSAGE_CLASS,
        Tag::ORIGINAL_MESSAGE_CLASS,
    ] {
        assert_eq!(writer.write_i32(tag, 5).unwrap_err().kind(), invalid);
    }
    for value in [0x8000, -0x8001] {
        assert_eq!(
            writer.write_i32(Tag::PRIORITY, value).unwrap_err().kind(),
            invalid
        );
    }
    for tag in [Tag::PRIORITY, Tag::DATE_SENT] {
        assert_eq!(writer.write_string(tag, "x").unwrap_err().kind(), invalid);
    }
    for tag in [Tag::MESSAGE_CLASS, Tag::ORIGINAL_MESSAGE_CLASS] {
        for text in [
            "",
            "IPM.\u{e9}",
            "IPM\0Note",
            "IPM\tNote",
            "IPM\u{7f}",
            &"x".repeat(255),
        ] {
            assert_eq!(writer.write_string(tag, text).unwrap_err().kind(), invalid);
        }
    }
    assert_eq!(
        writer
            .write_datetime(Tag::SUBJECT, date())
            .unwrap_err()
            .kind(),
        invalid
    );
    assert_eq!(
        writer
            .write_string(Tag::SUBJECT, "\u{1f600}")
            .unwrap_err()
            .kind(),
        invalid
    );
    assert!(!writer.is_faulted());
    assert!(writer.get_ref().is_empty());
    // None of the rejected values should have changed the writer's state.
    writer
        .write_string(Tag::MESSAGE_CLASS, &"x".repeat(254))
        .unwrap();
    writer
        .write_string(Tag::ORIGINAL_MESSAGE_CLASS, "IPM.Note")
        .unwrap();
    writer.write_string(Tag::SUBJECT, "").unwrap();
    writer.finish().unwrap();
    let values = decode(writer.get_ref());
    assert_eq!(values[2].2.len(), 255);
    assert_eq!(values[4].2, [0]);
}

#[test]
fn attribute_ordering_and_recovery_after_rejected_transitions() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let reject = |writer: &mut TnefWriter<Vec<u8>>, tag| {
        let before = writer.get_ref().clone();
        assert_eq!(
            writer.write_attribute(tag, &[]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!writer.is_faulted());
        assert_eq!(writer.get_ref(), &before);
    };
    // An attachment attribute requires attAttachRenderData first.
    reject(&mut writer, Tag::ATTACH_TITLE);
    reject(&mut writer, Tag::ATTACHMENT);
    writer.write_string(Tag::SUBJECT, "x").unwrap();
    writer
        .write_attribute(Tag::RECIPIENT_TABLE, &[0; 4])
        .unwrap();
    writer
        .write_attribute(Tag::MAPI_PROPERTIES, &[0; 4])
        .unwrap();
    // attMsgProps is the last message attribute.
    reject(&mut writer, Tag::SUBJECT);
    reject(&mut writer, Tag::MAPI_PROPERTIES);
    reject(&mut writer, Tag::RECIPIENT_TABLE);
    writer
        .write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    writer.write_string(Tag::ATTACH_TITLE, "a.txt").unwrap();
    // No message attributes after the first attachment.
    reject(&mut writer, Tag::PRIORITY);
    writer.write_attribute(Tag::ATTACHMENT, &[0; 4]).unwrap();
    // attAttachment is the last attribute of the attachment.
    reject(&mut writer, Tag::ATTACH_TITLE);
    reject(&mut writer, Tag::ATTACHMENT);
    reject(&mut writer, Tag::SUBJECT);
    writer
        .write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    writer.write_string(Tag::ATTACH_TITLE, "b.txt").unwrap();
    writer.finish().unwrap();
    let values = decode(writer.get_ref());
    assert_eq!(
        values.iter().skip(2).map(|v| v.1).collect::<Vec<_>>(),
        [
            Tag::SUBJECT,
            Tag::RECIPIENT_TABLE,
            Tag::MAPI_PROPERTIES,
            Tag::ATTACH_RENDER_DATA,
            Tag::ATTACH_TITLE,
            Tag::ATTACHMENT,
            Tag::ATTACH_RENDER_DATA,
            Tag::ATTACH_TITLE,
        ]
    );
    for item in &values[5..] {
        assert_eq!(item.0, Level::Attachment);
    }
}

#[test]
fn direct_and_streamed_checksums_wrap_at_sixteen_bits() {
    let payload = vec![255; 100 * 1024];
    for chunk in [1, 7, 4096, usize::MAX] {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        writer
            .write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])
            .unwrap();
        writer
            .write_attribute(Tag::ATTACH_DATA, &payload[..1024])
            .unwrap();
        writer.write_string(Tag::ATTACH_TITLE, "x").unwrap();
        let mut source = Source::new(&payload, chunk);
        writer
            .write_attribute_from(Tag::ATTACH_META_FILE, &mut source, payload.len() as u64)
            .unwrap();
        writer.finish().unwrap();
        let bytes = writer.into_inner();
        let expected_sum = ((100 * 1024 * 255) & 0xffff) as u16;
        assert_eq!(&bytes[bytes.len() - 2..], &expected_sum.to_le_bytes());
        assert_eq!(decode(&bytes).last().unwrap().2, payload);
    }
}

#[test]
fn streamed_values_are_bounded_and_empty_values_do_not_read() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut source = Source::new(b"abcNEXT", 1);
    source.interrupt = true;
    writer
        .write_attribute_from(Tag::BODY, &mut source, 3)
        .unwrap();
    assert_eq!(source.input, b"NEXT");
    let calls = source.calls;
    writer
        .write_attribute_from(Tag::BODY, &mut source, 0)
        .unwrap();
    assert_eq!(source.calls, calls);
    for length in [i32::MAX as u64 + 1, u64::MAX] {
        let before = writer.get_ref().clone();
        assert_eq!(
            writer
                .write_attribute_from(Tag::BODY, &mut source, length)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(source.calls, calls);
        assert_eq!(writer.get_ref(), &before);
        assert!(!writer.is_faulted());
    }
    writer
        .write_attribute_from(Tag::BODY, &mut source, 4)
        .unwrap();
    writer.finish().unwrap();
    let values = decode(writer.get_ref());
    assert_eq!(values[2].2, b"abc");
    assert!(values[3].2.is_empty());
    assert_eq!(values[4].2, b"NEXT");
}

#[test]
fn streamed_source_failures_are_terminal() {
    for failure in [false, true] {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut source = Source::new(b"abc", 1);
        source.fail_on_eof = failure;
        let error = writer
            .write_attribute_from(Tag::BODY, &mut source, 4)
            .unwrap_err();
        assert_eq!(
            error.kind(),
            if failure {
                io::ErrorKind::Other
            } else {
                io::ErrorKind::UnexpectedEof
            }
        );
        assert!(writer.is_faulted());
        let before = writer.get_ref().clone();
        let calls = source.calls;
        assert_eq!(
            writer
                .write_attribute_from(Tag::BODY, &mut source, 0)
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            writer.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(source.calls, calls);
        assert_eq!(writer.get_ref(), &before);
        assert_eq!(
            before.len(),
            40 + 9 + 3,
            "no checksum should have been emitted"
        );
    }
}

#[test]
fn streamed_sink_failures_do_not_retry_consumed_input() {
    let mut expected = header(1252, 0);
    attribute(&mut expected, 1, Tag::BODY, b"abcdefgh");
    for boundary in 0..expected.len() {
        let mut sink = Sink::new(7);
        sink.fail_after = Some(boundary);
        let calls = sink.calls.clone();
        let mut source = Source::new(b"abcdefghNEXT", 4096);
        let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
        assert_eq!(
            writer
                .write_attribute_from(Tag::BODY, &mut source, 8)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other,
        );
        assert_eq!(writer.get_ref().bytes, expected[..boundary]);
        if boundary < 49 {
            assert_eq!(
                source.calls, 0,
                "source consumed after a failed header write"
            );
        } else {
            assert_eq!(source.input, b"NEXT");
        }
        let reads = source.calls;
        let writes = calls.get();
        assert_eq!(
            writer
                .write_attribute_from(Tag::BODY, &mut source, 0)
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe,
        );
        assert_faulted(&mut writer);
        assert_eq!(source.calls, reads);
        assert_eq!(calls.get(), writes);
    }
}

#[test]
fn short_writes_and_interrupted_write_and_flush_are_retried() {
    for chunk in [1, 7, usize::MAX] {
        let mut sink = Sink::new(chunk);
        sink.interrupt_write = true;
        sink.interrupt_flush = true;
        let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
        writer.write_string(Tag::SUBJECT, "Hello").unwrap();
        writer.flush().unwrap();
        writer.write_string(Tag::BODY, "world").unwrap();
        writer.finish().unwrap();
        let mut expected = header(1252, 0);
        attribute(&mut expected, 1, Tag::SUBJECT, b"Hello\0");
        attribute(&mut expected, 1, Tag::BODY, b"world\0");
        assert_eq!(writer.into_inner().bytes, expected);
    }
}

#[test]
fn every_partial_write_boundary_and_zero_write_faults_without_retry() {
    // Exercise each byte of the automatic header, attribute header, value,
    // NUL terminator, and checksum, not only the first write to the sink.
    let mut expected = header(1252, 0);
    attribute(&mut expected, 1, Tag::SUBJECT, b"Hello\0");
    for zero in [false, true] {
        for boundary in 0..expected.len() {
            let mut sink = Sink::new(7);
            sink.fail_after = Some(boundary);
            sink.zero_write = zero;
            let calls = sink.calls.clone();
            let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
            let error = writer.write_string(Tag::SUBJECT, "Hello").unwrap_err();
            assert_eq!(
                error.kind(),
                if zero {
                    io::ErrorKind::WriteZero
                } else {
                    io::ErrorKind::Other
                }
            );
            assert!(writer.is_faulted());
            assert_eq!(writer.get_ref().bytes, expected[..boundary]);
            let before = calls.get();
            assert_faulted(&mut writer);
            drop(writer);
            assert_eq!(calls.get(), before);
        }
    }
}

#[test]
fn flush_failure_and_finish_state_are_explicit() {
    let sink = Sink::new(usize::MAX);
    sink.fail_flush.set(true);
    let calls = sink.calls.clone();
    let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
    assert_eq!(writer.finish().unwrap_err().kind(), io::ErrorKind::Other);
    assert!(!writer.is_finished());
    assert!(writer.is_faulted());
    assert_eq!(writer.get_ref().bytes, header(1252, 0));
    let before = calls.get();
    assert_faulted(&mut writer);
    let sink = writer.into_inner();
    assert_eq!(calls.get(), before);
    assert_eq!(sink.bytes.len(), 40);

    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    writer.finish().unwrap();
    let before = writer.get_ref().clone();
    assert_eq!(
        writer.write_attribute(Tag::BODY, &[]).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        writer.write_string(Tag::SUBJECT, "").unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        writer.write_i32(Tag::PRIORITY, 0).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        writer
            .write_datetime(Tag::DATE_SENT, date())
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    writer.finish().unwrap();
    assert!(writer.is_finished());
    assert!(!writer.is_faulted());
    assert_eq!(writer.get_ref(), &before);
}

#[test]
fn flush_failure_after_finish_is_also_terminal() {
    let sink = Sink::new(usize::MAX);
    let fail_flush = sink.fail_flush.clone();
    let calls = sink.calls.clone();
    let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
    writer.finish().unwrap();
    let finished_calls = calls.get();
    writer.finish().unwrap();
    assert_eq!(calls.get(), finished_calls);
    fail_flush.set(true);
    assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::Other);
    assert!(writer.is_faulted());
    assert!(!writer.is_finished());
    let faulted_calls = calls.get();
    assert_faulted(&mut writer);
    assert_eq!(calls.get(), faulted_calls);
}

#[test]
fn drop_and_into_inner_do_not_write_and_borrowed_sinks_remain_usable() {
    for finish in [false, true] {
        let sink = Sink::new(usize::MAX);
        let calls = sink.calls.clone();
        let mut writer = TnefWriter::new(sink, 1252, 0).unwrap();
        if finish {
            writer.finish().unwrap();
        }
        let before = calls.get();
        drop(writer);
        assert_eq!(calls.get(), before);
    }
    let writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    assert!(writer.into_inner().is_empty());
    let mut output = b"prefix".to_vec();
    {
        let mut writer = TnefWriter::new(&mut output, 1252, 0xbeef).unwrap();
        writer.finish().unwrap();
    }
    output.extend_from_slice(b"suffix");
    assert_eq!(&output[..6], b"prefix");
    assert_eq!(&output[6..46], header(1252, 0xbeef));
    assert_eq!(&output[46..], b"suffix");
}

fn assert_faulted(writer: &mut TnefWriter<Sink>) {
    let broken = io::ErrorKind::BrokenPipe;
    assert_eq!(
        writer.write_attribute(Tag::BODY, &[]).unwrap_err().kind(),
        broken
    );
    assert_eq!(
        writer
            .write_attribute_from(Tag::BODY, &mut &b""[..], 0)
            .unwrap_err()
            .kind(),
        broken
    );
    assert_eq!(
        writer.write_string(Tag::SUBJECT, "").unwrap_err().kind(),
        broken
    );
    assert_eq!(
        writer.write_i32(Tag::PRIORITY, 0).unwrap_err().kind(),
        broken
    );
    assert_eq!(
        writer
            .write_datetime(Tag::DATE_SENT, date())
            .unwrap_err()
            .kind(),
        broken
    );
    assert_eq!(writer.flush().unwrap_err().kind(), broken);
    assert_eq!(writer.finish().unwrap_err().kind(), broken);
}

struct Source<'a> {
    input: &'a [u8],
    chunk: usize,
    calls: usize,
    interrupt: bool,
    fail_on_eof: bool,
}

impl<'a> Source<'a> {
    fn new(input: &'a [u8], chunk: usize) -> Self {
        Self {
            input,
            chunk,
            calls: 0,
            interrupt: false,
            fail_on_eof: false,
        }
    }
}

impl Read for Source<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.input.is_empty() && self.fail_on_eof {
            return Err(io::Error::other("source failure"));
        }
        let count = buffer.len().min(self.chunk).min(self.input.len());
        buffer[..count].copy_from_slice(&self.input[..count]);
        self.input = &self.input[count..];
        Ok(count)
    }
}

#[derive(Debug)]
struct Sink {
    bytes: Vec<u8>,
    chunk: usize,
    fail_after: Option<usize>,
    zero_write: bool,
    interrupt_write: bool,
    interrupt_flush: bool,
    fail_flush: Rc<Cell<bool>>,
    calls: Rc<Cell<usize>>,
}

impl Sink {
    fn new(chunk: usize) -> Self {
        Self {
            bytes: Vec::new(),
            chunk,
            fail_after: None,
            zero_write: false,
            interrupt_write: false,
            interrupt_flush: false,
            fail_flush: Rc::new(Cell::new(false)),
            calls: Rc::new(Cell::new(0)),
        }
    }
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls.set(self.calls.get() + 1);
        if std::mem::take(&mut self.interrupt_write) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_after == Some(self.bytes.len()) {
            return if self.zero_write {
                Ok(0)
            } else {
                Err(io::Error::other("sink failure"))
            };
        }
        let available = self
            .fail_after
            .map_or(usize::MAX, |limit| limit - self.bytes.len());
        let count = bytes.len().min(self.chunk).min(available);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.calls.set(self.calls.get() + 1);
        if std::mem::take(&mut self.interrupt_flush) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_flush.get() {
            Err(io::Error::other("flush failure"))
        } else {
            Ok(())
        }
    }
}

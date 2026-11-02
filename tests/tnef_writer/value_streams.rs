// Adapted from MimeKit's TnefWriterTests value-stream, multivalue, and attachment cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    Tag as A, TnefWriter, attribute, header,
    properties::{read_sets, variable},
};
use mimecraft::tnef::{
    TnefAttachMethod, TnefMessage, TnefName, TnefNameId, TnefPropertyTag as P,
    TnefPropertyType as Type, TnefValue as V,
};
use std::io::{self, Cursor, Read, Write};

#[test]
fn streamed_binary_and_objects_match_independent_wire_at_every_padding_boundary() {
    for kind in [Type::BINARY, Type::OBJECT] {
        for length in [
            0, 1, 2, 3, 4, 5, 7, 8, 15, 16, 17, 2047, 2048, 2049, 100_000,
        ] {
            let length = length + if kind == Type::OBJECT { 16 } else { 0 };
            let data: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
            let tag = P::new(0x6000, kind);
            let mut payload = 1u32.to_le_bytes().to_vec();
            payload.extend_from_slice(&tag.0.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend(variable(&data));
            let mut expected = header(1252, 0);
            attribute(&mut expected, 1, A::MAPI_PROPERTIES, &payload);
            for chunk in [1, 7, 4096, length.max(1)] {
                let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
                let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
                properties.write_tag(tag).unwrap();
                let mut value = properties.open_value().unwrap();
                assert!(value.is_empty());
                assert_eq!(value.write(&[]).unwrap(), 0);
                let mut accepted = 0;
                for input in data.chunks(chunk) {
                    value.write_all(input).unwrap();
                    accepted += input.len();
                    assert_eq!(value.len(), accepted as u64);
                }
                value.flush().unwrap();
                assert!(!value.is_faulted());
                value.finish().unwrap();
                assert!(!properties.is_faulted());
                assert_eq!(properties.get_ref().len(), payload.len() as u64);
                assert!(properties.open_value().is_err(), "scalar already completed");
                properties.finish().unwrap();
                writer.finish().unwrap();
                assert_eq!(writer.get_ref(), &expected);
            }
            let sets = read_sets(&expected, A::MAPI_PROPERTIES);
            assert_eq!(
                sets[0][0].value(),
                Some(&if kind == Type::OBJECT {
                    V::Object(data)
                } else {
                    V::Binary(data)
                })
            );
        }
    }
}

#[test]
fn mixed_streamed_and_native_multivalues_include_empty_values_and_properties() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
    let tag = P::new(0x6000, Type(Type::BINARY.0 | Type::MULTI_VALUED.0));
    properties.write_tag(tag).unwrap();
    let mut value = properties.open_value().unwrap();
    value.write_all(&[1, 2, 3, 4, 5]).unwrap();
    value.finish().unwrap();
    properties.write_value(&V::Binary(vec![6])).unwrap();
    properties.open_value().unwrap().finish().unwrap();
    // An empty multi-valued property.
    properties
        .write_tag(P::new(0x6001, Type(Type::LONG.0 | Type::MULTI_VALUED.0)))
        .unwrap();
    properties.finish().unwrap();
    writer.finish().unwrap();
    let sets = read_sets(writer.get_ref(), A::MAPI_PROPERTIES);
    assert_eq!(
        sets[0][0].values(),
        [
            V::Binary(vec![1, 2, 3, 4, 5]),
            V::Binary(vec![6]),
            V::Binary(vec![])
        ]
    );
    assert!(sets[0][1].values().is_empty());
}

#[test]
fn named_streams_in_recipient_rows_reuse_ids_with_borrowed_scratch() {
    let name = TnefNameId {
        guid: [0x42; 16],
        name: TnefName::Id(42),
    };
    let mut scratch = Cursor::new(Vec::new());
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer
        .open_properties_with_buffer(A::RECIPIENT_TABLE, &mut scratch)
        .unwrap();
    for i in 0..3 {
        properties.begin_row().unwrap();
        properties.write_named_tag(&name, Type::BINARY).unwrap();
        let mut value = properties.open_value().unwrap();
        value.write_all(&[i; 3]).unwrap();
        value.finish().unwrap();
    }
    properties.finish().unwrap();
    assert_eq!(scratch.position(), scratch.get_ref().len() as u64);
    writer.finish().unwrap();
    let sets = read_sets(writer.get_ref(), A::RECIPIENT_TABLE);
    assert_eq!(sets.len(), 3);
    for (i, row) in sets.iter().enumerate() {
        assert_eq!(row[0].tag(), P::new(0x8000, Type::BINARY));
        assert_eq!(row[0].name(), Some(&name));
        assert_eq!(row[0].value(), Some(&V::Binary(vec![i as u8; 3])));
    }
}

#[test]
fn invalid_opens_preserve_scratch_and_allow_recovery() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
    assert_eq!(
        properties.open_value().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    for (kind, native) in [
        (Type::NULL, None),
        (Type::LONG, Some(V::I32(1))),
        (Type::UNICODE, Some(V::Text("x".into()))),
        (Type::STRING8, Some(V::Text("y".into()))),
        (Type::CLASS_ID, Some(V::Guid([0; 16]))),
    ] {
        properties.write_tag(P::new(0x6000, kind)).unwrap();
        let before = properties.get_ref().to_vec().unwrap();
        assert_eq!(
            properties.open_value().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(properties.get_ref().to_vec().unwrap(), before);
        assert!(!properties.is_faulted());
        if let Some(value) = native {
            properties.write_value(&value).unwrap();
        }
    }
    properties.write_tag(P::new(0x6001, Type::BINARY)).unwrap();
    properties.write_value(&V::Binary(vec![42])).unwrap();
    let before = properties.get_ref().to_vec().unwrap();
    assert!(properties.open_value().is_err());
    assert_eq!(properties.get_ref().to_vec().unwrap(), before);
    properties.finish().unwrap();
    writer.finish().unwrap();
}

#[test]
fn short_object_finish_is_terminal_and_does_not_emit_the_attribute() {
    for length in 0..16 {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        writer.write_string(A::SUBJECT, "before").unwrap();
        let before = writer.get_ref().clone();
        let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
        properties.write_tag(P::new(0x6000, Type::OBJECT)).unwrap();
        let mut value = properties.open_value().unwrap();
        value.write_all(&[0; 16][..length]).unwrap();
        assert_eq!(
            value.finish().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(properties.is_faulted());
        assert_eq!(
            properties.flush().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            properties.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(writer.is_faulted());
        // The faulted attribute is discarded; preceding attributes remain intact.
        assert_eq!(writer.get_ref(), &before);
    }
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "verify that forgetting a borrowed child cannot bypass completion"
)]
fn abandoned_and_forgotten_values_block_all_property_operations_even_for_empty_multivalues() {
    for forget in [false, true] {
        for length in [0, 5] {
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            let mut scratch = Cursor::new(Vec::new());
            let mut properties = writer
                .open_properties_with_buffer(A::MAPI_PROPERTIES, &mut scratch)
                .unwrap();
            properties
                .write_tag(P::new(0x6000, Type(Type::BINARY.0 | Type::MULTI_VALUED.0)))
                .unwrap();
            {
                let mut value = properties.open_value().unwrap();
                value.write_all(&[42; 5][..length]).unwrap();
                if forget {
                    std::mem::forget(value);
                }
            }
            let before = properties.get_ref().get_ref().clone();
            assert!(properties.is_faulted());
            assert_eq!(
                properties.open_value().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                properties
                    .write_value(&V::Binary(vec![]))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                properties.write_tag(P::SUBJECT_W).unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                properties.begin_row().unwrap_err().kind(),
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
            assert_eq!(scratch.get_ref(), &before);
            assert_eq!(
                writer.finish().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert!(writer.get_ref().is_empty());
        }
    }
}

#[test]
fn upstream_streamed_attachments_load_with_metadata_and_full_content() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    writer.write_string(A::MESSAGE_CLASS, "IPM.Note").unwrap();
    let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
    properties.write_tag(P::SUBJECT_W).unwrap();
    properties
        .write_value(&V::Text("With attachments".into()))
        .unwrap();
    properties.finish().unwrap();
    for i in 0..3 {
        writer
            .write_attribute(A::ATTACH_RENDER_DATA, &[0; 14])
            .unwrap();
        writer
            .write_string(A::ATTACH_TITLE, &format!("file{i}.txt"))
            .unwrap();
        let mut properties = writer.open_properties(A::ATTACHMENT).unwrap();
        for (tag, value) in [
            (P::ATTACH_METHOD, V::I32(1)),
            (
                P::ATTACH_LONG_FILENAME_W,
                V::Text(format!("long file name {i}.txt")),
            ),
            (P::ATTACH_MIME_TAG_A, V::Text("text/plain".into())),
        ] {
            properties.write_tag(tag).unwrap();
            properties.write_value(&value).unwrap();
        }
        properties.write_tag(P::ATTACH_DATA_BIN).unwrap();
        let mut value = properties.open_value().unwrap();
        value.write_all(b"attachment content").unwrap();
        value.write_all(&[b'0' + i]).unwrap();
        value.finish().unwrap();
        properties.finish().unwrap();
    }
    writer.finish().unwrap();
    let message = TnefMessage::load(writer.get_ref().as_slice()).unwrap();
    assert_eq!(
        message.message_class().unwrap().as_deref(),
        Some("IPM.Note")
    );
    assert_eq!(
        message.subject().unwrap().as_deref(),
        Some("With attachments")
    );
    assert_eq!(message.attachments().len(), 3);
    for (i, attachment) in message.attachments().iter().enumerate() {
        assert_eq!(
            attachment.file_name().unwrap().as_deref(),
            Some(format!("long file name {i}.txt").as_str())
        );
        assert_eq!(
            attachment.mime_type().unwrap().as_deref(),
            Some("text/plain")
        );
        assert_eq!(attachment.method().unwrap(), TnefAttachMethod::ByValue);
        let mut data = Vec::new();
        attachment.open_read().read_to_end(&mut data).unwrap();
        assert_eq!(data, format!("attachment content{i}").as_bytes());
    }
}

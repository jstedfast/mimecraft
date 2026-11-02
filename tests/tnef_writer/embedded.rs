// Adapted from MimeKit's UnitTests/Tnef/TnefWriterTests.cs embedded-message cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    Tag as A, TnefWriter, attribute, header,
    properties::{read_sets, variable},
};
use mimecraft::tnef::{
    TnefEmbeddedSink, TnefMessage, TnefName, TnefNameId, TnefPropertyTag as P,
    TnefPropertyType as Type, TnefValue as V,
};
use std::io::{self, Cursor, Write};

const IID: [u8; 16] = [7, 3, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];

#[test]
fn upstream_nested_messages_inherit_or_override_codepages() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0x1234).unwrap();
    writer.write_string(A::SUBJECT, "outer").unwrap();
    writer
        .write_attribute(A::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    let mut properties = writer.open_properties(A::ATTACHMENT).unwrap();
    properties.write_tag(P::ATTACH_METHOD).unwrap();
    properties.write_value(&V::I32(5)).unwrap();
    properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let mut embedded = properties
        .open_embedded_message_with_codepage(1251)
        .unwrap();
    assert_eq!(embedded.codepage(), 1251);
    assert_eq!(embedded.legacy_key(), 0);
    embedded.write_string(A::MESSAGE_CLASS, "IPM.Note").unwrap();
    let subject = "\u{438}\u{43d}\u{43d}\u{435}\u{440}";
    embedded.write_string(A::SUBJECT, subject).unwrap();
    let mut inner = embedded.open_properties(A::MAPI_PROPERTIES).unwrap();
    inner.write_tag(P::SUBJECT_A).unwrap();
    inner.write_value(&V::Text(subject.into())).unwrap();
    inner.write_tag(P::RTF_COMPRESSED).unwrap();
    let mut rtf = inner
        .open_rtf_compressed(mimecraft::tnef::RtfCompressionMode::Compressed)
        .unwrap();
    rtf.write_all(b"{\\rtf1 inner}").unwrap();
    rtf.finish().unwrap();
    inner.finish().unwrap();
    embedded
        .write_attribute(A::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    let mut inner = embedded.open_properties(A::ATTACHMENT).unwrap();
    inner.write_tag(P::ATTACH_METHOD).unwrap();
    inner.write_value(&V::I32(5)).unwrap();
    inner.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let mut nested = inner.open_embedded_message().unwrap();
    assert_eq!(nested.codepage(), 1251, "inherits the codepage");
    nested.write_string(A::SUBJECT, "nested").unwrap();
    nested.finish().unwrap();
    inner.finish().unwrap();
    embedded.finish().unwrap();
    properties.write_tag(P::ATTACH_LONG_FILENAME_W).unwrap();
    properties
        .write_value(&V::Text("inner.msg".into()))
        .unwrap();
    properties.finish().unwrap();
    writer.finish().unwrap();
    let message = TnefMessage::load(writer.get_ref().as_slice()).unwrap();
    let attachment = &message.attachments()[0];
    assert_eq!(
        attachment.file_name().unwrap().as_deref(),
        Some("inner.msg")
    );
    let inner = attachment.load_embedded_message().unwrap();
    assert_eq!(inner.codepage(), 1251);
    assert_eq!(inner.subject().unwrap().as_deref(), Some(subject));
    assert_eq!(inner.message_class().unwrap().as_deref(), Some("IPM.Note"));
    assert_eq!(inner.rtf_body().unwrap().text().unwrap(), "{\\rtf1 inner}");
    let nested = inner.attachments()[0].load_embedded_message().unwrap();
    assert_eq!(nested.subject().unwrap().as_deref(), Some("nested"));
}

#[test]
fn automatic_iid_empty_headers_and_padding_match_independent_wire_bytes() {
    for codepage in [1252, 1251, 65001] {
        for length in 0..8 {
            let mut nested = header(codepage, 0);
            if length != 0 {
                attribute(&mut nested, 1, A::BODY, &vec![b'x'; length]);
            }
            let mut object = IID.to_vec();
            object.extend_from_slice(&nested);
            let mut data: Vec<u8> = [3u32, P::IMPORTANCE.0, 42, P::ATTACH_DATA_OBJ.0, 1]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            data.extend(variable(&object));
            data.extend(P::IMPORTANCE.0.to_le_bytes());
            data.extend(43u32.to_le_bytes());
            let mut expected = header(1252, 9);
            attribute(&mut expected, 1, A::MAPI_PROPERTIES, &data);
            let mut writer = TnefWriter::new(Vec::new(), 1252, 9).unwrap();
            let mut scratch = Cursor::new(Vec::new());
            let mut properties = writer
                .open_properties_with_buffer(A::MAPI_PROPERTIES, &mut scratch)
                .unwrap();
            properties.write_tag(P::IMPORTANCE).unwrap();
            properties.write_value(&V::I32(42)).unwrap();
            properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
            let mut embedded = properties
                .open_embedded_message_with_codepage(codepage)
                .unwrap();
            if length != 0 {
                embedded
                    .write_attribute(A::BODY, &vec![b'x'; length])
                    .unwrap();
            }
            embedded.flush().unwrap();
            embedded.finish().unwrap();
            properties.write_tag(P::IMPORTANCE).unwrap();
            properties.write_value(&V::I32(43)).unwrap();
            properties.finish().unwrap();
            assert_eq!(scratch.get_ref(), &data);
            assert_eq!(scratch.position(), data.len() as u64);
            writer.finish().unwrap();
            assert_eq!(writer.get_ref(), &expected);
            assert_eq!(
                read_sets(writer.get_ref(), A::MAPI_PROPERTIES)[0][1].value(),
                Some(&V::Object(object))
            );
        }
    }
}

#[test]
fn invalid_codepages_and_property_types_leave_the_writer_usable() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
    assert!(properties.open_embedded_message().is_err());
    for (tag, value) in [
        (P::IMPORTANCE, V::I32(1)),
        (P::ATTACH_DATA_BIN, V::Binary(vec![])),
    ] {
        properties.write_tag(tag).unwrap();
        let before = properties.get_ref().to_vec().unwrap();
        assert_eq!(
            properties.open_embedded_message().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(properties.get_ref().to_vec().unwrap(), before);
        properties.write_value(&value).unwrap();
    }
    properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let before = properties.get_ref().to_vec().unwrap();
    for codepage in [0, 1200, 1201, 12000, 12001, u32::MAX] {
        assert_eq!(
            properties
                .open_embedded_message_with_codepage(codepage)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(properties.get_ref().to_vec().unwrap(), before);
        assert!(!properties.is_faulted());
    }
    // The property writer is still usable after an invalid codepage.
    properties
        .open_embedded_message()
        .unwrap()
        .finish()
        .unwrap();
    assert!(
        properties.open_embedded_message().is_err(),
        "completed scalar"
    );
    properties.finish().unwrap();
    writer.finish().unwrap();
}

#[test]
fn incomplete_nested_properties_fault_all_enclosing_writers() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
    properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let mut embedded = properties.open_embedded_message().unwrap();
    let mut inner = embedded.open_properties(A::MAPI_PROPERTIES).unwrap();
    inner.write_tag(P::SUBJECT_W).unwrap();
    assert_eq!(
        inner.finish().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(embedded.is_faulted());
    assert_eq!(
        embedded.finish().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        properties.finish().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(writer.get_ref().is_empty());
    super::assert_faulted(&mut writer);
}

#[test]
fn dropped_forgotten_and_inner_only_finished_messages_do_not_complete_values() {
    for forget in [false, true] {
        for finish_inner in [false, true] {
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
            properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
            {
                let mut embedded = properties.open_embedded_message().unwrap();
                if finish_inner {
                    // Finishing through DerefMut does not complete the Object.
                    TnefWriter::finish(&mut embedded).unwrap();
                }
                if forget {
                    std::mem::forget(embedded);
                }
            }
            let before = properties.get_ref().to_vec().unwrap();
            assert!(properties.is_faulted());
            assert_eq!(
                properties.open_embedded_message().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                properties.write_tag(P::SUBJECT_W).unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                properties.flush().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(properties.get_ref().to_vec().unwrap(), before);
            assert_eq!(
                properties.finish().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            super::assert_faulted(&mut writer);
        }
    }
}

fn recursive_message(writer: &mut TnefWriter<TnefEmbeddedSink<'_>>, depth: usize) {
    writer.write_string(A::SUBJECT, &depth.to_string()).unwrap();
    if depth == 0 {
        return;
    }
    writer
        .write_attribute(A::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    let mut properties = writer.open_properties(A::ATTACHMENT).unwrap();
    properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let mut embedded = properties.open_embedded_message().unwrap();
    recursive_message(&mut embedded, depth - 1);
    embedded.finish().unwrap();
    properties.finish().unwrap();
}

#[test]
fn runtime_recursion_uses_a_stable_sink_type_without_boxing() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    writer
        .write_attribute(A::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    let mut properties = writer.open_properties(A::ATTACHMENT).unwrap();
    properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let mut embedded = properties.open_embedded_message().unwrap();
    recursive_message(&mut embedded, 8);
    embedded.finish().unwrap();
    properties.finish().unwrap();
    writer.finish().unwrap();
    let mut message = TnefMessage::load(writer.get_ref().as_slice()).unwrap();
    for depth in (0..=8).rev() {
        message = message.attachments()[0].load_embedded_message().unwrap();
        assert_eq!(
            message.subject().unwrap().as_deref(),
            Some(depth.to_string().as_str())
        );
    }
    assert!(message.attachments().is_empty());
}

#[test]
fn named_object_recipient_values_and_nested_names_keep_stream_local_ids() {
    let first = TnefNameId {
        guid: [0x42; 16],
        name: TnefName::String("Outer".into()),
    };
    let shared = TnefNameId {
        guid: [0x42; 16],
        name: TnefName::String("Shared".into()),
    };
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(A::RECIPIENT_TABLE).unwrap();
    properties.begin_row().unwrap();
    properties.write_named_tag(&first, Type::LONG).unwrap();
    properties.write_value(&V::I32(1)).unwrap();
    properties.write_named_tag(&shared, Type::OBJECT).unwrap();
    let mut embedded = properties.open_embedded_message().unwrap();
    let mut nested = embedded.open_properties(A::MAPI_PROPERTIES).unwrap();
    nested.write_named_tag(&shared, Type::LONG).unwrap();
    nested.write_value(&V::I32(7)).unwrap();
    nested.finish().unwrap();
    embedded.finish().unwrap();
    properties.write_named_tag(&shared, Type::LONG).unwrap();
    properties.write_value(&V::I32(8)).unwrap();
    properties.finish().unwrap();
    writer.finish().unwrap();
    let sets = read_sets(writer.get_ref(), A::RECIPIENT_TABLE);
    assert_eq!(sets[0][0].tag(), P::new(0x8000, Type::LONG));
    assert_eq!(sets[0][1].tag(), P::new(0x8001, Type::OBJECT));
    assert_eq!(sets[0][2].tag(), P::new(0x8001, Type::LONG));
    let Some(V::Object(bytes)) = sets[0][1].value() else {
        panic!("not Object")
    };
    assert_eq!(bytes[..16], IID);
    let nested = read_sets(&bytes[16..], A::MAPI_PROPERTIES);
    assert_eq!(nested[0][0].tag(), P::new(0x8000, Type::LONG));
    assert_eq!(nested[0][0].name(), Some(&shared));
    assert_eq!(nested[0][0].value(), Some(&V::I32(7)));
}

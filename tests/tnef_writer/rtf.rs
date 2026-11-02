// Adapted from MimeKit's TnefWriterTests compressed-RTF body and multivalue cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    Tag as A, TnefWriter, attribute, header,
    properties::{read_sets, variable},
};
use mimecraft::tnef::{
    RtfCompressedToRtf, RtfCompressedWriter, RtfCompressionMode as Mode, TnefMessage, TnefName,
    TnefNameId, TnefPropertyTag as P, TnefPropertyType as Type, TnefValue as V,
};
use std::io::{self, Cursor, Write};

fn compress(input: &[u8], mode: Mode) -> Vec<u8> {
    let mut encoder = RtfCompressedWriter::new(Cursor::new(Vec::new()), mode).unwrap();
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap();
    encoder.into_inner().into_inner()
}

pub(super) fn decoded(bytes: &[u8], mode: Mode) -> Vec<u8> {
    let mut decoder = RtfCompressedToRtf::new();
    let mut result = Vec::new();
    for input in bytes.chunks(7) {
        decoder.filter(input, &mut result);
    }
    decoder.validate().unwrap();
    assert_eq!(decoder.compression_mode(), mode);
    assert!(decoder.is_valid_crc32());
    result
}

#[test]
fn upstream_rtf_body_round_trips_and_compressed_tnef_is_less_than_one_quarter() {
    let mut text = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0 Arial;}}");
    for _ in 0..500 {
        text.push_str("\\pard Some text that repeats.\\par\r\n");
    }
    text.push('}');
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
        properties.write_tag(P::RTF_COMPRESSED).unwrap();
        let mut rtf = properties.open_rtf_compressed(mode).unwrap();
        assert_eq!(rtf.raw_size(), 0);
        assert_eq!(rtf.compression_mode(), mode);
        assert_eq!(rtf.write(&[]).unwrap(), 0);
        rtf.write_all(text.as_bytes()).unwrap();
        assert_eq!(rtf.raw_size(), text.len() as u64);
        assert!(!rtf.is_faulted());
        rtf.finish().unwrap();
        assert!(!properties.is_faulted());
        assert!(
            properties.open_rtf_compressed(mode).is_err(),
            "completed scalar"
        );
        properties.finish().unwrap();
        writer.finish().unwrap();
        if mode == Mode::Compressed {
            assert!(writer.get_ref().len() < text.len() / 4);
        }
        let message = TnefMessage::load(writer.get_ref().as_slice()).unwrap();
        assert_eq!(message.rtf_body().unwrap().text().unwrap(), text);
        let sets = read_sets(writer.get_ref(), A::MAPI_PROPERTIES);
        let Some(V::Binary(bytes)) = sets[0][0].value() else {
            panic!("not binary")
        };
        assert_eq!(decoded(bytes, mode), text.as_bytes());
    }
}

#[test]
fn header_patches_preserve_surrounding_properties_at_every_split_and_flush() {
    let input = b"{\\rtf1\\ansi\\ansicpg1252\\pard hello world}\r\n";
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        let encoded = compress(input, mode);
        let mut data = 3u32.to_le_bytes().to_vec();
        data.extend_from_slice(&P::IMPORTANCE.0.to_le_bytes());
        data.extend_from_slice(&42i32.to_le_bytes());
        data.extend_from_slice(&P::RTF_COMPRESSED.0.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend(variable(&encoded));
        data.extend_from_slice(&P::IMPORTANCE.0.to_le_bytes());
        data.extend_from_slice(&43i32.to_le_bytes());
        let mut expected = header(1252, 0);
        attribute(&mut expected, 1, A::MAPI_PROPERTIES, &data);
        for split in 0..=input.len() {
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            let mut scratch = Cursor::new(Vec::new());
            let mut properties = writer
                .open_properties_with_buffer(A::MAPI_PROPERTIES, &mut scratch)
                .unwrap();
            properties.write_tag(P::IMPORTANCE).unwrap();
            properties.write_value(&V::I32(42)).unwrap();
            properties.write_tag(P::RTF_COMPRESSED).unwrap();
            let mut rtf = properties.open_rtf_compressed(mode).unwrap();
            rtf.write_all(&input[..split]).unwrap();
            rtf.flush().unwrap();
            rtf.write_all(&input[split..]).unwrap();
            rtf.flush().unwrap();
            rtf.finish().unwrap();
            properties.write_tag(P::IMPORTANCE).unwrap();
            properties.write_value(&V::I32(43)).unwrap();
            properties.finish().unwrap();
            assert_eq!(scratch.position(), data.len() as u64);
            assert_eq!(scratch.get_ref(), &data);
            writer.finish().unwrap();
            assert_eq!(writer.get_ref(), &expected);
        }
    }
}

#[test]
fn upstream_multivalued_rtf_supports_independent_encoders_and_native_values() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
    properties
        .write_tag(P::new(0x6000, Type(Type::BINARY.0 | Type::MULTI_VALUED.0)))
        .unwrap();
    for i in 0..2 {
        let mut rtf = properties.open_rtf_compressed(Mode::Compressed).unwrap();
        rtf.write_all(format!("{{\\rtf1 value {i}}}").as_bytes())
            .unwrap();
        rtf.finish().unwrap();
    }
    properties.write_value(&V::Binary(vec![1, 2, 3])).unwrap();
    properties.finish().unwrap();
    writer.finish().unwrap();
    let sets = read_sets(writer.get_ref(), A::MAPI_PROPERTIES);
    let values = sets[0][0].values();
    assert_eq!(values.len(), 3);
    for (i, value) in values[..2].iter().enumerate() {
        let V::Binary(bytes) = value else {
            panic!("not binary")
        };
        assert_eq!(
            decoded(bytes, Mode::Compressed),
            format!("{{\\rtf1 value {i}}}").as_bytes()
        );
    }
    assert_eq!(values[2], V::Binary(vec![1, 2, 3]));
}

#[test]
fn empty_values_and_binary_tails_cover_padding_and_dictionary_boundaries() {
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
        properties
            .write_tag(P::new(0x6000, Type(Type::BINARY.0 | Type::MULTI_VALUED.0)))
            .unwrap();
        let mut expected = Vec::new();
        for length in (0..20).chain([4095, 4096, 4097, 8193]) {
            let data: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
            let mut rtf = properties.open_rtf_compressed(mode).unwrap();
            for input in data.chunks(7) {
                rtf.write_all(input).unwrap();
            }
            rtf.finish().unwrap();
            expected.push(data);
        }
        properties.finish().unwrap();
        writer.finish().unwrap();
        let sets = read_sets(writer.get_ref(), A::MAPI_PROPERTIES);
        assert_eq!(sets[0][0].values().len(), expected.len());
        for (value, data) in sets[0][0].values().iter().zip(expected) {
            let V::Binary(bytes) = value else {
                panic!("not binary")
            };
            assert_eq!(bytes, &compress(&data, mode));
            assert_eq!(decoded(bytes, mode), data);
        }
    }
}

#[test]
fn invalid_modes_and_tags_leave_properties_usable_without_writing_headers() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
    assert!(properties.open_rtf_compressed(Mode::Compressed).is_err());
    properties.write_tag(P::IMPORTANCE).unwrap();
    let before = properties.get_ref().to_vec().unwrap();
    assert!(properties.open_rtf_compressed(Mode::Compressed).is_err());
    assert_eq!(properties.get_ref().to_vec().unwrap(), before);
    properties.write_value(&V::I32(1)).unwrap();
    properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
    let before = properties.get_ref().to_vec().unwrap();
    assert!(properties.open_rtf_compressed(Mode::Compressed).is_err());
    assert_eq!(properties.get_ref().to_vec().unwrap(), before);
    properties.write_value(&V::Object(vec![0; 16])).unwrap();
    properties.write_tag(P::RTF_COMPRESSED).unwrap();
    let before = properties.get_ref().to_vec().unwrap();
    for mode in [Mode::Unknown(0), Mode::Unknown(u32::from(Mode::Compressed))] {
        assert_eq!(
            properties.open_rtf_compressed(mode).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!properties.is_faulted());
        assert_eq!(properties.get_ref().to_vec().unwrap(), before);
    }
    // The property writer is still usable.
    properties
        .open_rtf_compressed(Mode::Compressed)
        .unwrap()
        .finish()
        .unwrap();
    properties.finish().unwrap();
    writer.finish().unwrap();
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "verify forgotten borrowed encoders cannot bypass completion"
)]
fn abandoned_and_forgotten_encoders_never_complete_empty_multivalues() {
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        for forget in [false, true] {
            let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
            writer.write_string(A::SUBJECT, "prior").unwrap();
            let before = writer.get_ref().clone();
            let mut properties = writer.open_properties(A::MAPI_PROPERTIES).unwrap();
            properties
                .write_tag(P::new(0x6000, Type(Type::BINARY.0 | Type::MULTI_VALUED.0)))
                .unwrap();
            {
                let rtf = properties.open_rtf_compressed(mode).unwrap();
                if forget {
                    std::mem::forget(rtf);
                }
            }
            assert!(properties.is_faulted());
            assert_eq!(
                properties.open_rtf_compressed(mode).unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
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
            assert_eq!(writer.get_ref(), &before);
            super::assert_faulted(&mut writer);
        }
    }
}

#[test]
fn named_rtf_in_recipient_rows_and_attachment_properties_uses_shared_storage() {
    let name = TnefNameId {
        guid: [0x42; 16],
        name: TnefName::Id(7),
    };
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    for attr in [A::RECIPIENT_TABLE, A::MAPI_PROPERTIES, A::ATTACHMENT] {
        if attr == A::ATTACHMENT {
            writer
                .write_attribute(A::ATTACH_RENDER_DATA, &[0; 14])
                .unwrap();
        }
        let mut properties = writer.open_properties(attr).unwrap();
        if attr == A::RECIPIENT_TABLE {
            properties.begin_row().unwrap();
        }
        properties.write_named_tag(&name, Type::BINARY).unwrap();
        let mut rtf = properties.open_rtf_compressed(Mode::Uncompressed).unwrap();
        rtf.write_all(b"{\\rtf1 example}").unwrap();
        rtf.finish().unwrap();
        properties.finish().unwrap();
    }
    writer.finish().unwrap();
    for attr in [A::RECIPIENT_TABLE, A::MAPI_PROPERTIES, A::ATTACHMENT] {
        let sets = read_sets(writer.get_ref(), attr);
        assert_eq!(sets[0][0].tag(), P::new(0x8000, Type::BINARY));
        assert_eq!(sets[0][0].name(), Some(&name));
        let Some(V::Binary(bytes)) = sets[0][0].value() else {
            panic!("not binary")
        };
        assert_eq!(decoded(bytes, Mode::Uncompressed), b"{\\rtf1 example}");
    }
}

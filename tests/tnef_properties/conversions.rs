// Adapted from MimeKit's TnefPropertySetTests and TnefPropertyReaderCoercionTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::borrow::Cow;

use super::{
    sets::{multi, read_set},
    *,
};
use mimecraft::{
    charsets::Charset,
    tnef::{
        TnefDateTime, TnefDateTimeError, TnefDateTimeKind as Kind, TnefProperty, TnefPropertySet,
        TnefValueConversionError as ConversionError,
    },
};

#[test]
fn core_tag_constants_match_upstream_wire_tags() {
    let cases = [
        (Tag::ADDRTYPE_A, 0x3002001e),
        (Tag::ADDRTYPE_W, 0x3002001f),
        (Tag::ATTACH_CONTENT_ID_A, 0x3712001e),
        (Tag::ATTACH_CONTENT_ID_W, 0x3712001f),
        (Tag::ATTACH_DATA_BIN, 0x37010102),
        (Tag::ATTACH_DATA_OBJ, 0x3701000d),
        (Tag::ATTACH_FILENAME_A, 0x3704001e),
        (Tag::ATTACH_FILENAME_W, 0x3704001f),
        (Tag::ATTACH_FLAGS, 0x37140003),
        (Tag::ATTACH_LONG_FILENAME_A, 0x3707001e),
        (Tag::ATTACH_LONG_FILENAME_W, 0x3707001f),
        (Tag::ATTACH_METHOD, 0x37050003),
        (Tag::ATTACH_MIME_TAG_A, 0x370e001e),
        (Tag::ATTACH_MIME_TAG_W, 0x370e001f),
        (Tag::BODY_A, 0x1000001e),
        (Tag::BODY_W, 0x1000001f),
        (Tag::CLIENT_SUBMIT_TIME, 0x00390040),
        (Tag::DISPLAY_NAME_A, 0x3001001e),
        (Tag::DISPLAY_NAME_W, 0x3001001f),
        (Tag::EMAIL_ADDRESS_A, 0x3003001e),
        (Tag::EMAIL_ADDRESS_W, 0x3003001f),
        (Tag::IMPORTANCE, 0x00170003),
        (Tag::INTERNET_CODEPAGE, 0x3fde0003),
        (Tag::INTERNET_MESSAGE_ID_A, 0x1035001e),
        (Tag::INTERNET_MESSAGE_ID_W, 0x1035001f),
        (Tag::MESSAGE_CLASS_A, 0x001a001e),
        (Tag::MESSAGE_CLASS_W, 0x001a001f),
        (Tag::MESSAGE_DELIVERY_TIME, 0x0e060040),
        (Tag::PRIORITY, 0x00260003),
        (Tag::RECIPIENT_DISPLAY_NAME_A, 0x5ff6001e),
        (Tag::RECIPIENT_DISPLAY_NAME_W, 0x5ff6001f),
        (Tag::RECIPIENT_TYPE, 0x0c150003),
        (Tag::RTF_COMPRESSED, 0x10090102),
        (Tag::SMTP_ADDRESS_A, 0x39fe001e),
        (Tag::SMTP_ADDRESS_W, 0x39fe001f),
        (Tag::SUBJECT_A, 0x0037001e),
        (Tag::SUBJECT_W, 0x0037001f),
        (Tag::TRANSMITABLE_DISPLAY_NAME_A, 0x3a20001e),
        (Tag::TRANSMITABLE_DISPLAY_NAME_W, 0x3a20001f),
    ];
    for (tag, wire) in cases {
        assert_eq!(tag.0, wire);
        assert!(!tag.is_multi_valued());
        assert!(!tag.is_named());
    }
}

#[test]
fn upstream_owned_value_type_conversions_preserve_native_snapshots() {
    let guid = [
        0x33, 0x22, 0x11, 0, 0x55, 0x44, 0x77, 0x66, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
    ];
    let mut body = text(Type::UNICODE, &unicode("Unicode body\0"));
    body[..4].copy_from_slice(&Tag::BODY_W.0.to_le_bytes());
    let mut binary = text(Type::BINARY, &[1, 2, 3, 4, 5]);
    binary[..4].copy_from_slice(&Tag::ATTACH_DATA_BIN.0.to_le_bytes());
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            fixed(Tag::IMPORTANCE.id(), Type::I2, &0x1234i32.to_le_bytes()),
            fixed(Tag::PRIORITY.id(), Type::LONG, &42i32.to_le_bytes()),
            fixed(0x6601, Type::R4, &1.5f32.to_le_bytes()),
            fixed(0x6602, Type::DOUBLE, &2.25f64.to_le_bytes()),
            fixed(0x6603, Type::CURRENCY, &123456789i64.to_le_bytes()),
            fixed(0x6604, Type::APP_TIME, &2.25f64.to_le_bytes()),
            fixed(0x6605, Type::ERROR, &0x8004010fu32.to_le_bytes()),
            fixed(0x6606, Type::BOOLEAN, &1i32.to_le_bytes()),
            fixed(0x6607, Type::I8, &0x0123456789abcdefi64.to_le_bytes()),
            fixed(
                Tag::CLIENT_SUBMIT_TIME.id(),
                Type::SYS_TIME,
                &1i64.to_le_bytes(),
            ),
            fixed(0x6608, Type::CLASS_ID, &guid),
            text(Type::STRING8, b"8-bit subject\0"),
            body,
            binary,
        ]),
    );
    for chunk in [1, 7, 4096] {
        let set = read_set(&data, chunk);
        let original = set.clone();
        for _ in 0..2 {
            assert_eq!(set.len(), 14);
            assert_eq!(set[0].to_i16(), Ok(0x1234));
            assert_eq!(set.get_i16(Tag::IMPORTANCE), Ok(Some(0x1234)));
            assert_eq!(set[0].to_i32(), Ok(0x1234));
            assert_eq!(set.get_i32(Tag::PRIORITY), Ok(Some(42)));
            assert_eq!(set.get_i64(Tag::PRIORITY), Ok(Some(42)));
            assert_eq!(set.get_f64(Tag::PRIORITY), Ok(Some(42.0)));
            assert_eq!(set.get_bool(Tag::PRIORITY), Ok(Some(true)));
            assert_eq!(set[2].to_f32(), Ok(1.5));
            assert_eq!(set.get_f32(set[2].tag()), Ok(Some(1.5)));
            assert_eq!(set[2].to_f64(), Ok(1.5));
            assert_eq!(set[3].to_i32(), Ok(2));
            assert_eq!(set[4].to_f64(), Ok(12345.6789));
            assert_eq!(set[4].to_i64(), Ok(12345));
            assert_eq!(
                set[5].to_datetime(),
                Ok(TnefDateTime::new(1900, 1, 1, 6, 0, 0, Kind::Unspecified).unwrap())
            );
            assert_eq!(set[6].to_i64(), Ok(i64::from(0x8004010fu32 as i32)));
            assert_eq!(set[6].value(), Some(&Value::Error(0x8004010f)));
            assert_eq!(set[7].to_i32(), Ok(1));
            assert_eq!(set[8].to_i64(), Ok(0x0123456789abcdef));
            let date = set.get_datetime(Tag::CLIENT_SUBMIT_TIME).unwrap().unwrap();
            assert_eq!((date.year(), date.month(), date.day()), (1601, 1, 1));
            assert_eq!(date.nanosecond(), 100);
            assert_eq!(date.kind(), Kind::Utc);
            assert_eq!(set.get_guid(set[10].tag()), Ok(Some(guid)));
            assert_eq!(set[10].as_bytes(), Ok(guid.as_slice()));
            assert_eq!(
                set.get_text(Tag::SUBJECT_W).unwrap().as_deref(),
                Some("8-bit subject")
            );
            assert_eq!(
                set.get_text(Tag::BODY_W).unwrap().as_deref(),
                Some("Unicode body")
            );
            assert_eq!(
                set.get_bytes(Tag::ATTACH_DATA_BIN),
                Ok(Some([1, 2, 3, 4, 5].as_slice()))
            );
            assert_eq!(set[13].to_text().unwrap(), "\u{1}\u{2}\u{3}\u{4}\u{5}");
            assert_eq!(set[11].to_i32(), Err(ConversionError::InvalidType));
            assert_eq!(set[11].to_datetime(), Err(ConversionError::InvalidType));
            assert_eq!(set[11].to_guid(), Err(ConversionError::InvalidType));
            assert_eq!(set[11].as_bytes(), Err(ConversionError::InvalidType));
            assert_eq!(set[1].to_text(), Err(ConversionError::InvalidType));
            assert_eq!(set.get_bool(Tag::BODY_W), Err(ConversionError::InvalidType));
            assert_eq!(
                set.get_i64(Tag::ATTACH_DATA_BIN),
                Err(ConversionError::InvalidType)
            );
            assert_eq!(
                set.get_guid(Tag::PRIORITY),
                Err(ConversionError::InvalidType)
            );
            assert_eq!(set.get_text(Tag::DISPLAY_NAME_W), Ok(None));
            assert_eq!(set, original);
        }
    }
}

#[test]
fn numeric_coercion_matrix_and_currency_fractions() {
    let cases = [
        (Value::I16(2), Some(true), 2, 2.0),
        (Value::Bool(true), Some(true), 1, 1.0),
        (Value::I32(2), Some(true), 2, 2.0),
        (Value::Error(2), Some(true), 2, 2.0),
        (Value::I64(2), Some(true), 2, 2.0),
        (Value::Currency(25000), Some(true), 2, 2.5),
        (Value::Currency(-25000), Some(true), -2, -2.5),
        (Value::Currency(1), Some(true), 0, 0.0001),
        (Value::Currency(-1), Some(true), 0, -0.0001),
        (Value::Currency(0), Some(false), 0, 0.0),
        (Value::F32(2.5), None, 2, 2.5),
        (Value::F64(-2.5), None, -2, -2.5),
        (Value::Bool(false), Some(false), 0, 0.0),
        (Value::I16(0), Some(false), 0, 0.0),
        (Value::I32(0), Some(false), 0, 0.0),
        (Value::I64(0), Some(false), 0, 0.0),
        (Value::Error(0), Some(false), 0, 0.0),
        (Value::Error(u32::MAX), Some(true), -1, -1.0),
    ];
    for (value, boolean, integer, float) in cases {
        assert_eq!(
            value.to_bool(),
            boolean.ok_or(ConversionError::InvalidType),
            "{value:?}"
        );
        assert_eq!(value.to_i16(), Ok(integer as i16), "{value:?}");
        assert_eq!(value.to_i32(), Ok(integer as i32), "{value:?}");
        assert_eq!(value.to_i64(), Ok(integer), "{value:?}");
        assert_eq!(value.to_f32(), Ok(float as f32), "{value:?}");
        assert_eq!(value.to_f64(), Ok(float), "{value:?}");
        assert_eq!(value.as_bytes(), Err(ConversionError::InvalidType));
        assert_eq!(value.to_guid(), Err(ConversionError::InvalidType));
    }
    for (raw, integer) in [(i64::MIN, -922337203685477), (i64::MAX, 922337203685477)] {
        assert_eq!(Value::Currency(raw).to_i64(), Ok(integer));
        assert_eq!(Value::Currency(raw).to_bool(), Ok(true));
    }
}

#[test]
fn integer_narrowing_keeps_low_bits_and_float_boundaries_are_explicit() {
    for (raw, i16_value, i32_value) in [
        (32768, i16::MIN, 32768),
        (65535, -1, 65535),
        (2147483648, 0, i32::MIN),
        (4294967295, -1, -1),
        (-65537, -1, -65537),
        (i64::MIN, 0, 0),
        (i64::MAX, -1, -1),
    ] {
        assert_eq!(Value::I64(raw).to_i16(), Ok(i16_value));
        assert_eq!(Value::I64(raw).to_i32(), Ok(i32_value));
    }
    for (raw, expected) in [
        (-0.0, 0),
        (0.0, 0),
        (0.999, 0),
        (-0.999, 0),
        (-2.999, -2),
        (-9223372036854775808.0, i64::MIN),
        (f64::from_bits(0x43dfffffffffffff), 9223372036854774784),
    ] {
        assert_eq!(Value::F64(raw).to_i64(), Ok(expected));
    }
    for value in [
        Value::F64(f64::NAN),
        Value::F64(f64::INFINITY),
        Value::F64(f64::NEG_INFINITY),
        Value::F64(9223372036854775808.0),
        Value::F64(f64::from_bits(0xc3e0000000000001)),
        Value::F32(f32::NAN),
        Value::F32(f32::INFINITY),
        Value::F32(f32::NEG_INFINITY),
        Value::F32(9223372036854775808.0),
        Value::F32(f32::from_bits(0xdf000001)),
    ] {
        assert_eq!(value.to_i16(), Err(ConversionError::OutOfRange));
        assert_eq!(value.to_i32(), Err(ConversionError::OutOfRange));
        assert_eq!(value.to_i64(), Err(ConversionError::OutOfRange));
    }
    assert_eq!(Value::F32(-9223372036854775808.0).to_i64(), Ok(i64::MIN));
    assert!(Value::F64(f64::NAN).to_f32().unwrap().is_nan());
    assert!(Value::F32(f32::NAN).to_f64().unwrap().is_nan());
    assert_eq!(Value::F64(f64::MAX).to_f32(), Ok(f32::INFINITY));
    assert_eq!(Value::F64(-f64::MAX).to_f32(), Ok(f32::NEG_INFINITY));
    assert_eq!(
        Value::F64(-0.0).to_f32().unwrap().to_bits(),
        (-0.0f32).to_bits()
    );
}

#[test]
fn dates_and_other_nonnumeric_types_are_not_coerced_to_numbers() {
    for value in [
        Value::Null,
        Value::Text("42".into()),
        Value::Binary(vec![42]),
        Value::Object(vec![42; 16]),
        Value::Guid([42; 16]),
        Value::AppTime(2.0),
        Value::SysTime(2),
    ] {
        assert_eq!(value.to_bool(), Err(ConversionError::InvalidType));
        assert_eq!(value.to_i16(), Err(ConversionError::InvalidType));
        assert_eq!(value.to_i32(), Err(ConversionError::InvalidType));
        assert_eq!(value.to_i64(), Err(ConversionError::InvalidType));
        assert_eq!(value.to_f32(), Err(ConversionError::InvalidType));
        assert_eq!(value.to_f64(), Err(ConversionError::InvalidType));
    }
}

fn assert_scalar_error(property: &TnefProperty, error: ConversionError) {
    assert_eq!(property.to_bool(), Err(error));
    assert_eq!(property.to_i16(), Err(error));
    assert_eq!(property.to_i32(), Err(error));
    assert_eq!(property.to_i64(), Err(error));
    assert_eq!(property.to_f32(), Err(error));
    assert_eq!(property.to_f64(), Err(error));
    assert_eq!(property.to_datetime(), Err(error));
    assert_eq!(property.to_guid(), Err(error));
    assert_eq!(property.as_bytes(), Err(error));
    assert_eq!(property.to_text(), Err(error));
}

#[test]
fn scalar_accessors_reject_multivalues_without_discarding_their_native_values() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            multi(Type::LONG, &[&1i32.to_le_bytes()]),
            multi(Type::LONG, &[&1i32.to_le_bytes(), &2i32.to_le_bytes()]),
            multi(Type::LONG, &[]),
            multi(Type::UNICODE, &[&unicode("one\0")]),
            multi(Type::BINARY, &[b"two"]),
        ]),
    );
    for chunk in [1, 7, 4096] {
        let set = read_set(&data, chunk);
        for property in &set {
            assert_scalar_error(property, ConversionError::MultiValued);
        }
        assert_eq!(set[0].values()[0].to_i32(), Ok(1));
        assert_eq!(set[1].values()[1].to_i64(), Ok(2));
        assert_eq!(set[2].values(), &[]);
        assert_eq!(set.get_i32(set[0].tag()), Err(ConversionError::MultiValued));
    }
}

#[test]
fn absent_empty_null_and_invalid_first_duplicates_remain_distinct() {
    let empty_set = TnefPropertySet::new();
    assert_eq!(empty_set.get_bool(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_i16(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_i32(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_i64(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_f32(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_f64(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_datetime(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_guid(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_bytes(Tag::PRIORITY), Ok(None));
    assert_eq!(empty_set.get_text(Tag::PRIORITY), Ok(None));

    let mut empty = fixed(Tag::SUBJECT_W.id(), Type::BINARY, &[]);
    dword(&mut empty, 0);
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            empty,
            text(Type::UNICODE, &unicode("later")),
            fixed(Tag::PRIORITY.id(), Type::NULL, &[]),
            fixed(Tag::PRIORITY.id(), Type::LONG, &7i32.to_le_bytes()),
        ]),
    );
    let mut reader = TnefReader::new(data.as_slice());
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let set = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_property_set()
        .unwrap();
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_scalar_error(&set[0], ConversionError::NoValue);
    assert_scalar_error(&set[2], ConversionError::InvalidType);
    assert_eq!(set.get_text(Tag::SUBJECT_W), Err(ConversionError::NoValue));
    assert_eq!(
        set.get_i32(Tag::PRIORITY),
        Err(ConversionError::InvalidType)
    );
    assert_eq!(set[1].to_text().unwrap(), "later");
    assert_eq!(set[3].to_i32(), Ok(7));
}

#[test]
fn captured_binary_charset_survives_codepage_changes_clone_and_reader_drop() {
    let set = {
        let data = document(
            Attribute::MAPI_PROPERTIES,
            &properties(&[
                text(Type::BINARY, b"Caf\xe9\0\0"),
                fixed(
                    Tag::INTERNET_CODEPAGE.id(),
                    Type::LONG,
                    &65001i32.to_le_bytes(),
                ),
                text(Type::BINARY, b"Caf\xc3\xa9\0"),
                text(Type::BINARY, b"\xff\0"),
                fixed(
                    Tag::INTERNET_CODEPAGE.id(),
                    Type::LONG,
                    &1252i32.to_le_bytes(),
                ),
                text(Type::BINARY, b"\x80\0"),
            ]),
        );
        read_set(&data, 1)
    };
    let clone = set.clone();
    drop(set);
    for _ in 0..2 {
        assert_eq!(
            clone[0].charset(),
            Charset::from_label("windows-1252").unwrap()
        );
        assert_eq!(clone[2].charset(), Charset::UTF8);
        assert_eq!(clone[0].to_text().unwrap(), "Caf\u{e9}");
        assert_eq!(clone[2].to_text().unwrap(), "Caf\u{e9}");
        assert_eq!(clone[3].to_text().unwrap(), "\u{fffd}");
        assert_eq!(clone[5].to_text().unwrap(), "\u{20ac}");
        assert_eq!(clone[0].as_bytes().unwrap(), b"Caf\xe9\0\0");
    }
}

#[test]
fn text_and_binary_views_borrow_and_preserve_embedded_nuls_and_object_iids() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            text(Type::UNICODE, &unicode("text\0")),
            text(Type::BINARY, b"ascii\0middle\0\0"),
            text(Type::BINARY, b"\0\0"),
            text(Type::BINARY, b""),
            text(Type::OBJECT, &[7; 20]),
            fixed(0x1234, Type::CLASS_ID, &[0x42; 16]),
        ]),
    );
    let set = read_set(&data, 7);
    assert!(matches!(set[0].to_text().unwrap(), Cow::Borrowed("text")));
    assert!(matches!(
        set[1].to_text().unwrap(),
        Cow::Borrowed("ascii\0middle")
    ));
    assert_eq!(set[2].to_text().unwrap(), "");
    assert_eq!(set[3].to_text().unwrap(), "");
    assert_eq!(set[4].to_text(), Err(ConversionError::InvalidType));
    assert_eq!(set[4].as_bytes().unwrap(), &[7; 20]);
    for property in &set.as_slice()[1..] {
        let native = match property.value().unwrap() {
            Value::Binary(bytes) | Value::Object(bytes) => bytes.as_slice(),
            Value::Guid(bytes) => bytes.as_slice(),
            _ => unreachable!(),
        };
        assert!(std::ptr::eq(property.as_bytes().unwrap(), native));
    }
}

#[test]
fn date_errors_do_not_repeat_diagnostics_or_fault_the_reader() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            fixed(0x6601, Type::APP_TIME, &f64::INFINITY.to_le_bytes()),
            fixed(0x6602, Type::SYS_TIME, &(-1i64).to_le_bytes()),
            fixed(Tag::PRIORITY.id(), Type::LONG, &7i32.to_le_bytes()),
        ]),
    );
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let set = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_property_set()
        .unwrap();
    let error = ConversionError::InvalidDate(TnefDateTimeError::OutOfRange);
    for _ in 0..2 {
        assert_eq!(set[0].to_datetime(), Err(error));
        assert_eq!(set.get_datetime(set[1].tag()), Err(error));
        assert_eq!(set.get_i32(Tag::PRIORITY), Ok(Some(7)));
    }
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert!(!reader.is_faulted());
    assert_eq!(issues.len(), 2);
    assert!(
        issues
            .iter()
            .all(|issue| issue.violation == Violation::InvalidDate)
    );
    assert_eq!(set[0].value(), Some(&Value::AppTime(f64::INFINITY)));
    assert_eq!(set[1].value(), Some(&Value::SysTime(-1)));
    assert!(std::error::Error::source(&error).is_some());
    for error in [
        error,
        ConversionError::InvalidType,
        ConversionError::OutOfRange,
        ConversionError::MultiValued,
        ConversionError::NoValue,
    ] {
        assert!(!error.to_string().is_empty());
    }
}

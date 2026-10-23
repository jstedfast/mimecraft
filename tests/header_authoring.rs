// Authoring vectors adapted from MimeKit's ParameterTests.cs,
// ParameterListTests.cs, ContentTypeTests.cs and ContentDispositionTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentDisposition, ContentDispositionSizeError, ContentHeaderParseErrorKind, ContentType,
    DateTimeOffset, HeaderParser, Parameter, ParameterList, ParameterListEditError,
    ParameterParseErrorKind, charsets::Charset,
};

fn pairs(list: &ParameterList) -> Vec<(&str, &str)> {
    list.into_iter()
        .map(|parameter| (parameter.name(), parameter.value()))
        .collect()
}

#[test]
fn parameters_store_decoded_values_literally() {
    for value in [
        "",
        "plain",
        "\"quoted\"",
        "=?utf-8?q?caf=C3=A9?=",
        "%C3%A9",
        "caf\u{e9}\u{1f642}",
        "a\r\n\tb\0c",
        r"..\file.txt",
    ] {
        let mut parameter = Parameter::new("FileName", value).unwrap();
        assert_eq!(parameter.name(), "FileName");
        assert_eq!(parameter.value(), value);
        let original = parameter.clone();
        parameter.set_value(format!("{value}-changed"));
        assert_eq!(original.value(), value);
        assert_eq!(parameter.value(), format!("{value}-changed"));
        assert_eq!(parameter.name(), "FileName");
    }
}

#[test]
fn parameter_name_validation_checks_every_byte_class() {
    for byte in 0u8..=255 {
        let allowed = byte.is_ascii_alphanumeric() || b"!#$&+-.^_`{|}~".contains(&byte);
        let name = format!("x{}y", char::from(byte));
        let result = Parameter::new(&name, "value");
        assert_eq!(result.is_ok(), allowed, "byte {byte}");
        if let Err(error) = result {
            assert_eq!(error.kind(), ParameterParseErrorKind::InvalidName);
            assert_eq!(error.offset(), 1);
        }
    }
    for (name, offset) in [
        ("", 0),
        ("filename*", 8),
        ("filename*0", 8),
        ("a b", 1),
        ("X-\u{6d4b}\u{8bd5}", 2),
        ("name\r\nInjected", 4),
    ] {
        let error = Parameter::new(name, "value").unwrap_err();
        assert_eq!(error.kind(), ParameterParseErrorKind::InvalidName);
        assert_eq!(error.offset(), offset);
        let mut list = ParameterList::new();
        list.set("existing", "unchanged").unwrap();
        let before = list.clone();
        assert_eq!(list.set(name, "value").unwrap_err(), error);
        assert_eq!(list, before);
    }
}

#[test]
fn ordered_case_insensitive_list_operations() {
    let mut list = ParameterList::new();
    assert_eq!(list, ParameterList::default());
    list.add(Parameter::new("abc", "0").unwrap()).unwrap();
    list.set("def", "1").unwrap();
    list.set("ghi", "2").unwrap();
    assert_eq!(pairs(&list), [("abc", "0"), ("def", "1"), ("ghi", "2")]);
    for (name, index) in [("aBc", 0), ("DEf", 1), ("gHI", 2)] {
        assert!(list.contains(name));
        assert_eq!(list.index_of(name), Some(index));
        assert_eq!(list.get(name), Some(index.to_string().as_str()));
    }
    assert!(!list.contains("xyz"));
    assert_eq!(list.index_of("xyz"), None);
    assert_eq!(list.remove("xyz"), None);
    list.insert(0, Parameter::new("xyz", "3").unwrap()).unwrap();
    let removed = list.remove("XYZ").unwrap();
    assert_eq!(removed, Parameter::new("xyz", "3").unwrap());
    list.insert(list.len(), removed).unwrap();
    assert_eq!(list.remove_at(3).unwrap().name(), "xyz");
    assert_eq!(list.set("ABC", "replaced").unwrap(), Some("0".into()));
    assert_eq!(
        pairs(&list),
        [("abc", "replaced"), ("def", "1"), ("ghi", "2")]
    );
    let old = list
        .replace(0, Parameter::new("AbC", "again").unwrap())
        .unwrap();
    assert_eq!(old.name(), "abc");
    assert_eq!(old.value(), "replaced");
    assert_eq!(list.as_slice()[0].name(), "AbC");
    list.replace(0, Parameter::new("xxx", "0").unwrap())
        .unwrap();
    assert!(!list.contains("abc"));
    assert!(list.contains("XXX"));
    assert_eq!(
        list.iter().rev().map(Parameter::name).collect::<Vec<_>>(),
        ["ghi", "def", "xxx"]
    );
    list.clear();
    assert!(list.is_empty());
    list.insert(0, Parameter::new("first", "value").unwrap())
        .unwrap();
    assert_eq!(pairs(&list), [("first", "value")]);
}

#[test]
fn duplicate_and_index_failures_are_atomic() {
    let mut list = ParameterList::parse(b"a=first; b*=unknown''%FF").unwrap();
    let before = list.clone();
    let duplicate = ParameterListEditError::DuplicateName("A".into());
    assert_eq!(
        list.add(Parameter::new("A", "changed").unwrap()),
        Err(duplicate.clone())
    );
    assert_eq!(list, before);
    assert_eq!(
        list.insert(1, Parameter::new("A", "changed").unwrap()),
        Err(duplicate.clone())
    );
    assert_eq!(list, before);
    assert_eq!(
        list.replace(1, Parameter::new("A", "changed").unwrap()),
        Err(duplicate)
    );
    assert_eq!(list, before);
    for index in [list.len() + 1, usize::MAX] {
        let error = ParameterListEditError::IndexOutOfBounds {
            index,
            len: list.len(),
        };
        assert_eq!(
            list.insert(index, Parameter::new("c", "x").unwrap()),
            Err(error.clone())
        );
        assert_eq!(
            list.replace(index, Parameter::new("c", "x").unwrap()),
            Err(error.clone())
        );
        assert_eq!(list.remove_at(index), Err(error));
        assert_eq!(list, before);
    }
    let error = ParameterListEditError::IndexOutOfBounds { index: 2, len: 2 };
    assert_eq!(
        list.replace(2, Parameter::new("c", "x").unwrap()),
        Err(error.clone())
    );
    assert_eq!(list.remove_at(2), Err(error));
    assert_eq!(list, before);
}

#[test]
fn edits_keep_original_diagnostics_and_clones_are_independent() {
    let original = ParameterList::parse(b" FileName*=unknown''%E9; FILENAME=ignored").unwrap();
    assert!(!original.issues().is_empty());
    let mut list = original.clone();
    list.set("FILENAME", "replacement").unwrap();
    list.add(Parameter::new("new", "value").unwrap()).unwrap();
    list.replace(1, Parameter::new("another", "value").unwrap())
        .unwrap();
    list.remove_at(1).unwrap();
    assert_eq!(list.issues(), original.issues());
    assert_eq!(original.get("filename"), Some("\u{e9}"));
    assert_eq!(list.as_slice()[0].name(), "FileName");
    list.clear();
    assert!(list.is_empty());
    assert_eq!(list.issues(), original.issues());
    assert_eq!(original.len(), 1);
}

#[test]
fn content_type_construction_and_component_setters() {
    let mut value = ContentType::new("application", "octet-stream").unwrap();
    value.set_name(Some("document.txt"));
    value.set_media_type("Text").unwrap();
    value.set_media_subtype("Plain").unwrap();
    assert_eq!(value.mime_type(), "Text/Plain");
    assert!(value.is_mime_type("text", "plain"));
    assert_eq!(value.name(), Some("document.txt"));
    for (token, offset) in [
        ("", 0),
        ("text/plain", 4),
        ("text ", 4),
        ("x\r\nY", 1),
        ("x\u{e9}", 1),
    ] {
        let before = value.clone();
        let error = value.set_media_type(token).unwrap_err();
        assert_eq!(error.kind(), ContentHeaderParseErrorKind::InvalidMediaType);
        assert_eq!(error.offset(), offset);
        assert_eq!(value, before);
        assert_eq!(ContentType::new(token, "plain").unwrap_err(), error);
    }
    for (token, offset) in [
        ("", 0),
        ("plain; charset=x", 5),
        ("a b", 1),
        ("a=b", 1),
        ("x\u{e9}", 1),
    ] {
        let before = value.clone();
        let error = value.set_media_subtype(token).unwrap_err();
        assert_eq!(
            error.kind(),
            ContentHeaderParseErrorKind::InvalidMediaSubtype
        );
        assert_eq!(error.offset(), offset);
        assert_eq!(value, before);
        assert_eq!(ContentType::new("text", token).unwrap_err(), error);
    }
    let value = ContentType::new("multipart", "multipart/mixed").unwrap();
    assert_eq!(
        ContentType::parse(value.mime_type().as_bytes()).unwrap(),
        value
    );
}

#[test]
fn authored_tokens_match_parser_alphabets() {
    for byte in 0u8..128 {
        let token = format!("x{}y", char::from(byte));
        let atom = byte.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&byte);
        let media_type = atom && byte != b'/';
        let media_subtype = (33..127).contains(&byte) && !b"()<>@,;:\\\"[]?=".contains(&byte);
        let result = ContentType::new(&token, "plain");
        assert_eq!(result.is_ok(), media_type, "media type byte {byte}");
        if let Ok(value) = result {
            assert_eq!(
                ContentType::parse(value.mime_type().as_bytes()).unwrap(),
                value
            );
        }
        let result = ContentType::new("application", &token);
        assert_eq!(result.is_ok(), media_subtype, "subtype byte {byte}");
        if let Ok(value) = result {
            assert_eq!(
                ContentType::parse(value.mime_type().as_bytes()).unwrap(),
                value
            );
        }
        let result = ContentDisposition::new(&token);
        assert_eq!(result.is_ok(), atom, "disposition byte {byte}");
        if let Ok(value) = result {
            assert_eq!(
                ContentDisposition::parse(value.disposition().as_bytes()).unwrap(),
                value
            );
        }
    }
}

#[test]
fn content_type_parameter_setters_preserve_names_and_removals() {
    let mut value = ContentType::parse(b"text/plain; CHARSET=unknown; Boundary=old; x=1").unwrap();
    value.set_charset_encoding(Some(Charset::UTF8));
    assert_eq!(value.charset_encoding(), Ok(Some(Charset::UTF8)));
    assert_eq!(value.charset(), Some(Charset::UTF8.name()));
    value.set_boundary(Some("new"));
    value.set_format(Some("flowed"));
    value.set_name(Some("caf\u{e9}.txt"));
    assert_eq!(
        value
            .parameters()
            .iter()
            .map(Parameter::name)
            .collect::<Vec<_>>(),
        ["CHARSET", "Boundary", "x", "format", "name"]
    );
    assert_eq!(value.boundary(), Some("new"));
    assert_eq!(value.format(), Some("flowed"));
    assert_eq!(value.name(), Some("caf\u{e9}.txt"));
    value.set_charset(Some("not-a-charset"));
    assert!(value.charset_encoding().is_err());
    value.set_charset_encoding(Some(Charset::LATIN1));
    assert_eq!(value.charset_encoding(), Ok(Some(Charset::LATIN1)));
    value.set_charset_encoding(None);
    value.set_boundary(None);
    value.set_format(None);
    value.set_name(None);
    assert_eq!(pairs(value.parameters()), [("x", "1")]);
    value.set_charset(Some(""));
    assert_eq!(value.charset(), Some(""));
    value.set_charset(None);
    assert_eq!(value.charset_encoding(), Ok(None));
    value.parameters_mut().set("Custom", "one").unwrap();
    assert_eq!(value.parameters().get("custom"), Some("one"));
}

#[test]
fn disposition_construction_and_setters_preserve_parameters() {
    let mut value = ContentDisposition::default();
    assert_eq!(value, ContentDisposition::new("attachment").unwrap());
    assert!(value.is_attachment());
    assert!(value.parameters().is_empty());
    value.set_file_name(Some("document.doc"));
    value.set_is_attachment(false);
    assert_eq!(value.disposition(), "inline");
    value.set_disposition("FORM-DATA").unwrap();
    assert_eq!(value.disposition(), "FORM-DATA");
    assert!(!value.is_attachment());
    value.set_is_attachment(true);
    assert_eq!(value.file_name(), Some("document.doc"));
    for (token, offset) in [
        ("", 0),
        ("inline ", 6),
        ("\"inline\"", 0),
        ("a;b", 1),
        ("a.b", 1),
        ("x\0y", 1),
        ("x\u{1f642}", 1),
    ] {
        let before = value.clone();
        let error = value.set_disposition(token).unwrap_err();
        assert_eq!(
            error.kind(),
            ContentHeaderParseErrorKind::InvalidDisposition
        );
        assert_eq!(error.offset(), offset);
        assert_eq!(value, before);
        assert_eq!(ContentDisposition::new(token).unwrap_err(), error);
    }
    let literal = "=?utf-8?q?name?=\r\n\0..\u{e9}";
    value.set_file_name(Some(literal));
    assert_eq!(value.file_name(), Some(literal));
    value.set_file_name(None);
    assert_eq!(value.file_name(), None);
    value.parameters_mut().set("Name", "upload").unwrap();
    assert_eq!(value.parameters().get("name"), Some("upload"));
}

#[test]
fn typed_date_and_size_setters_format_and_remove() {
    let creation = DateTimeOffset::new(1997, 1, 4, 15, 22, 17, -240).unwrap();
    let modification = DateTimeOffset::new(2007, 1, 4, 15, 22, 17, 330).unwrap();
    let read = DateTimeOffset::new(2012, 1, 4, 15, 22, 17, 0).unwrap();
    let mut value =
        ContentDisposition::parse(b"inline; Creation-Date=invalid; SIZE=not-a-number; x=unchanged")
            .unwrap();
    value.set_creation_date(Some(creation));
    value.set_modification_date(Some(modification));
    value.set_read_date(Some(read));
    value.set_size(Some(37001)).unwrap();
    assert_eq!(value.creation_date(), Ok(Some(creation)));
    assert_eq!(value.modification_date(), Ok(Some(modification)));
    assert_eq!(value.read_date(), Ok(Some(read)));
    assert_eq!(
        value.parameters().get("creation-date"),
        Some("Sat, 04 Jan 1997 15:22:17 -0400")
    );
    assert_eq!(
        value.parameters().get("modification-date"),
        Some("Thu, 04 Jan 2007 15:22:17 +0530")
    );
    assert_eq!(
        value.parameters().get("read-date"),
        Some("Wed, 04 Jan 2012 15:22:17 +0000")
    );
    assert_eq!(value.parameters().as_slice()[0].name(), "Creation-Date");
    assert_eq!(value.parameters().as_slice()[1].name(), "SIZE");
    assert_eq!(value.size(), Ok(Some(37001)));
    assert_eq!(value.parameters().get("size"), Some("37001"));
    for size in [0, i64::MAX as u64] {
        value.set_size(Some(size)).unwrap();
        assert_eq!(value.size(), Ok(Some(size)));
        assert_eq!(
            value.parameters().get("size"),
            Some(size.to_string().as_str())
        );
    }
    let before = value.clone();
    for size in [i64::MAX as u64 + 1, u64::MAX] {
        assert_eq!(
            value.set_size(Some(size)),
            Err(ContentDispositionSizeError::Overflow)
        );
        assert_eq!(value, before);
    }
    value.set_creation_date(None);
    value.set_modification_date(None);
    value.set_read_date(None);
    value.set_size(None).unwrap();
    assert_eq!(value.creation_date(), Ok(None));
    assert_eq!(value.modification_date(), Ok(None));
    assert_eq!(value.read_date(), Ok(None));
    assert_eq!(value.size(), Ok(None));
    assert_eq!(pairs(value.parameters()), [("x", "unchanged")]);
}

#[test]
fn parsed_header_edits_leave_raw_bytes_and_diagnostics_unchanged() {
    let input = b"Content-Type: text/plain; name*=unknown''%FF\r\n\
Content-Disposition: inline; filename*=unknown''%E9\r\n\r\nbody";
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    let consumed = parser.parse(input, &mut headers).unwrap();
    assert_eq!(&input[consumed..], b"body");
    let raw: Vec<_> = headers.iter().map(|header| header.raw().to_vec()).collect();
    let mut content_type = ContentType::parse(headers[0].raw_value()).unwrap();
    let mut disposition = ContentDisposition::parse(headers[1].raw_value()).unwrap();
    let original_type = content_type.clone();
    let original_disposition = disposition.clone();
    content_type.set_media_type("application").unwrap();
    content_type.set_media_subtype("octet-stream").unwrap();
    content_type.set_name(Some("new.bin"));
    disposition.set_disposition("attachment").unwrap();
    disposition.set_file_name(Some("new.bin"));
    assert_eq!(original_type.name(), Some("\u{ff}"));
    assert_eq!(original_disposition.file_name(), Some("\u{e9}"));
    assert!(!original_type.parameters().issues().is_empty());
    assert!(!original_disposition.parameters().issues().is_empty());
    assert_eq!(
        content_type.parameters().issues(),
        original_type.parameters().issues()
    );
    assert_eq!(
        disposition.parameters().issues(),
        original_disposition.parameters().issues()
    );
    for (header, original) in headers.iter().zip(raw) {
        assert_eq!(header.raw(), original);
    }
}

#[test]
fn repeated_list_edits_keep_names_unique_and_ordered() {
    let mut list = ParameterList::new();
    for index in 0..1000 {
        list.set(&format!("X{index}"), index.to_string()).unwrap();
    }
    for index in (0..1000).rev() {
        assert_eq!(
            list.set(&format!("x{index}"), "updated").unwrap(),
            Some(index.to_string())
        );
        assert_eq!(list.index_of(&format!("x{index}")), Some(index));
    }
    for index in (0..1000).step_by(2) {
        assert_eq!(
            list.remove(&format!("x{index}")).unwrap().name(),
            format!("X{index}")
        );
    }
    assert_eq!(list.len(), 500);
    for (position, parameter) in list.iter().enumerate() {
        assert_eq!(parameter.name(), format!("X{}", position * 2 + 1));
        assert_eq!(parameter.value(), "updated");
    }
}

// Adapted from MimeKit's UnitTests/Text/HtmlTagIdTests.cs,
// HtmlAttributeTests.cs, HtmlAttributeCollectionTests.cs, and HtmlWriterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{
    HtmlAttribute, HtmlAttributeCollection, HtmlAttributeError, HtmlAttributeId, HtmlTagId,
    HtmlWriter, HtmlWriterError, HtmlWriterState,
};

#[test]
fn identifiers_recognize_mixed_case_and_canonical_names() {
    for name in ["alt", "Alt", "aLt", "ALT", "AlT"] {
        assert_eq!(HtmlAttributeId::from_name(name), HtmlAttributeId::Alt);
    }
    for name in ["font", "FONT", "FoNt"] {
        assert_eq!(HtmlTagId::from_name(name), HtmlTagId::Font);
    }
    assert_eq!(HtmlTagId::from_name("a"), HtmlTagId::A);
    assert_eq!(HtmlTagId::from_name("A"), HtmlTagId::A);
    assert_eq!(HtmlTagId::Image.as_str(), "img");
    assert_eq!(HtmlTagId::Comment.as_str(), "!");
    assert_eq!(HtmlAttributeId::AcceptCharset.as_str(), "accept-charset");
    assert_eq!(HtmlAttributeId::HttpEquiv.as_str(), "http-equiv");
    assert_eq!(HtmlAttributeId::XmlNs.as_str(), "xmlns");
}

#[test]
fn unknown_names_are_not_trimmed_decoded_or_treated_as_markup() {
    for name in [
        "",
        "unknown",
        "x-custom",
        " img",
        "img ",
        "<img>",
        "im\u{212a}",
        "\0img",
        "&excl;",
        "\u{ff01}",
        "!\u{e9}",
    ] {
        let expected = if name.starts_with('!') {
            HtmlTagId::Comment
        } else {
            HtmlTagId::Unknown
        };
        assert_eq!(HtmlTagId::from_name(name), expected, "{name:?}");
    }
    for name in [
        "",
        "unknown",
        "data-custom",
        " href",
        "href ",
        "href=",
        "HREF\0",
        "!alt",
        "acceptcharset",
        "http_equiv",
        "h\u{212a}ref",
    ] {
        assert_eq!(
            HtmlAttributeId::from_name(name),
            HtmlAttributeId::Unknown,
            "{name:?}"
        );
    }
    assert_eq!(HtmlTagId::default(), HtmlTagId::Unknown);
    assert_eq!(HtmlAttributeId::default(), HtmlAttributeId::Unknown);
    assert_eq!(HtmlTagId::Unknown.as_str(), "Unknown");
    assert_eq!(HtmlAttributeId::Unknown.to_string(), "Unknown");
}

#[test]
fn every_leading_bang_is_a_comment_identifier() {
    for name in ["!", "!blah", "!DOCTYPE html", "!--", "!\0", "!\u{1f600}"] {
        assert_eq!(HtmlTagId::from_name(name), HtmlTagId::Comment, "{name:?}");
    }
}

#[test]
fn checked_attributes_preserve_names_and_unescaped_values() {
    let attribute = HtmlAttribute::new("HREF", Some("a&b\"<c>".into())).unwrap();
    assert_eq!(attribute.name(), "HREF");
    assert_eq!(attribute.id(), HtmlAttributeId::Href);
    assert_eq!(attribute.value(), Some("a&b\"<c>"));
    let canonical = HtmlAttribute::from_id(HtmlAttributeId::Href, Some("a&b\"<c>".into())).unwrap();
    assert_eq!(canonical.name(), "href");
    assert_eq!(canonical.id(), attribute.id());
    assert_eq!(canonical.value(), attribute.value());
    assert_ne!(canonical, attribute);
    assert_eq!(attribute.clone(), attribute);

    for name in ["data-Custom", "xml:lang", "\u{e9}tat", "Unknown"] {
        let attribute = HtmlAttribute::new(name, None).unwrap();
        assert_eq!(attribute.name(), name);
        assert_eq!(attribute.id(), HtmlAttributeId::Unknown);
        assert_eq!(attribute.value(), None);
    }
}

#[test]
fn valueless_and_empty_attributes_remain_distinct() {
    let valueless = HtmlAttribute::from_id(HtmlAttributeId::Checked, None).unwrap();
    let empty = HtmlAttribute::from_id(HtmlAttributeId::Checked, Some(String::new())).unwrap();
    assert_eq!(valueless.name(), empty.name());
    assert_eq!(valueless.value(), None);
    assert_eq!(empty.value(), Some(""));
    assert_ne!(valueless, empty);
}

#[test]
fn invalid_attribute_construction_reports_explicit_errors() {
    assert_eq!(
        HtmlAttribute::new("", None),
        Err(HtmlAttributeError::InvalidName)
    );
    assert_eq!(
        HtmlAttribute::new("a b c", None),
        Err(HtmlAttributeError::InvalidName)
    );
    assert_eq!(
        HtmlAttribute::from_id(HtmlAttributeId::Unknown, None),
        Err(HtmlAttributeError::UnknownId)
    );
    assert_eq!(
        HtmlAttribute::from_id(HtmlAttributeId::Unknown, Some(String::new())),
        Err(HtmlAttributeError::UnknownId)
    );
    for error in [
        HtmlAttributeError::InvalidName,
        HtmlAttributeError::UnknownId,
    ] {
        let error: &dyn std::error::Error = &error;
        assert!(!error.to_string().is_empty());
        assert!(error.source().is_none());
    }
}

#[test]
fn attribute_name_validation_matches_every_ascii_delimiter() {
    let invalid = b"\0\t\r\n\x0c /=>\"'<";
    for byte in 0..=127u8 {
        let ch = char::from(byte);
        for name in [
            ch.to_string(),
            format!("{ch}x"),
            format!("x{ch}"),
            format!("x{ch}y"),
        ] {
            let result = HtmlAttribute::new(name.clone(), None);
            assert_eq!(result.is_err(), invalid.contains(&byte), "{name:?}");
            if let Ok(attribute) = result {
                assert_eq!(attribute.name(), name);
            } else {
                assert_eq!(result, Err(HtmlAttributeError::InvalidName));
            }
        }
    }
}

#[test]
fn names_do_not_have_an_implicit_length_limit() {
    let name = format!("data-{}", "x".repeat(100_000));
    let attribute = HtmlAttribute::new(name.clone(), None).unwrap();
    assert_eq!(attribute.name(), name);
    assert_eq!(attribute.id(), HtmlAttributeId::Unknown);
    assert_eq!(HtmlTagId::from_name(&name), HtmlTagId::Unknown);
    let attributes: HtmlAttributeCollection = vec![attribute].into();
    assert!(attributes.contains(&name.to_ascii_uppercase()));
}

#[test]
fn empty_collections_support_every_lookup_and_iteration_path() {
    for attributes in [
        HtmlAttributeCollection::new(),
        HtmlAttributeCollection::default(),
        Vec::new().into(),
        std::iter::empty().collect(),
    ] {
        assert_eq!(attributes.len(), 0);
        assert!(attributes.is_empty());
        assert!(attributes.as_slice().is_empty());
        assert_eq!(attributes.iter().next(), None);
        assert_eq!((&attributes).into_iter().next(), None);
        for name in ["", "alt", "data-custom"] {
            assert!(!attributes.contains(name));
            assert_eq!(attributes.index_of(name), None);
            assert_eq!(attributes.get(name), None);
        }
        for id in [HtmlAttributeId::Alt, HtmlAttributeId::Unknown] {
            assert!(!attributes.contains_id(id));
            assert_eq!(attributes.index_of_id(id), None);
            assert_eq!(attributes.get_by_id(id), None);
        }
        assert!(attributes.clone().into_vec().is_empty());
        assert_eq!(attributes.into_iter().next(), None);
    }
}

#[test]
fn source_collection_lookups_and_enumeration() {
    let items = vec![
        HtmlAttribute::from_id(HtmlAttributeId::Alt, Some("This is some alt text.".into()))
            .unwrap(),
        HtmlAttribute::from_id(HtmlAttributeId::Text, Some("And this is the text.".into()))
            .unwrap(),
    ];
    let attributes: HtmlAttributeCollection = items.clone().into();
    assert_eq!(attributes.len(), 2);
    assert!(!attributes.is_empty());
    for (index, item) in items.iter().enumerate() {
        assert!(attributes.contains(item.name()));
        assert!(attributes.contains_id(item.id()));
        assert_eq!(attributes.index_of(item.name()), Some(index));
        assert_eq!(attributes.index_of_id(item.id()), Some(index));
        assert_eq!(attributes.get(item.name()), Some(item));
        assert_eq!(attributes.get_by_id(item.id()), Some(item));
        assert_eq!(&attributes[index], item);
    }
    assert!(!attributes.contains("background"));
    assert!(!attributes.contains_id(HtmlAttributeId::Background));
    assert_eq!(attributes.index_of("background"), None);
    assert_eq!(attributes.index_of_id(HtmlAttributeId::Background), None);
    assert_eq!(attributes.get("background"), None);
    assert_eq!(attributes.get_by_id(HtmlAttributeId::Background), None);
    assert_eq!(attributes.as_slice().get(2), None);
    assert_eq!(attributes.as_slice().get(usize::MAX), None);
    assert_eq!(attributes.as_slice(), items);
    assert_eq!(attributes.iter().next_back(), items.last());
    assert_eq!(
        (&attributes).into_iter().cloned().collect::<Vec<_>>(),
        items
    );
    assert_eq!(attributes.clone().into_iter().collect::<Vec<_>>(), items);
    assert_eq!(attributes.clone().into_vec(), items);
    assert_eq!(
        items.into_iter().collect::<HtmlAttributeCollection>(),
        attributes
    );
}

#[test]
fn collection_lookup_preserves_duplicates_and_returns_first_even_without_a_value() {
    let attributes: HtmlAttributeCollection = vec![
        HtmlAttribute::new("ALT", None).unwrap(),
        HtmlAttribute::from_id(HtmlAttributeId::Alt, Some("second".into())).unwrap(),
        HtmlAttribute::new("data-Custom", Some(String::new())).unwrap(),
        HtmlAttribute::new("DATA-custom", Some("last".into())).unwrap(),
        HtmlAttribute::new("other", None).unwrap(),
    ]
    .into();
    assert_eq!(attributes.len(), 5);
    assert_eq!(attributes.index_of("aLt"), Some(0));
    assert_eq!(attributes.index_of_id(HtmlAttributeId::Alt), Some(0));
    assert_eq!(attributes.get("alt"), Some(&attributes[0]));
    assert_eq!(
        attributes.get_by_id(HtmlAttributeId::Alt),
        Some(&attributes[0])
    );
    assert_eq!(attributes[0].value(), None);
    assert_eq!(attributes.index_of("DATA-CUSTOM"), Some(2));
    assert_eq!(attributes.get("data-custom"), Some(&attributes[2]));
    assert_eq!(
        attributes.get_by_id(HtmlAttributeId::Unknown),
        Some(&attributes[2])
    );
    assert_eq!(attributes.index_of_id(HtmlAttributeId::Unknown), Some(2));
    assert!(attributes.contains_id(HtmlAttributeId::Unknown));
    assert_eq!(attributes[2].value(), Some(""));
    assert_eq!(attributes[3].name(), "DATA-custom");
    assert_eq!(attributes[3].value(), Some("last"));
    for name in ["", " ALT", "alt ", "data-", "alt=", "ALT\0"] {
        assert!(!attributes.contains(name), "{name:?}");
        assert_eq!(attributes.index_of(name), None);
        assert_eq!(attributes.get(name), None);
    }
}

#[test]
fn collection_case_folding_is_ascii_only() {
    let attributes: HtmlAttributeCollection = vec![
        HtmlAttribute::new("data-\u{e9}", None).unwrap(),
        HtmlAttribute::new("id", None).unwrap(),
    ]
    .into();
    assert!(attributes.contains("DATA-\u{e9}"));
    assert!(!attributes.contains("data-\u{c9}"));
    assert!(!attributes.contains("\u{131}d"));
}

#[test]
#[should_panic]
fn positional_indexing_rejects_out_of_bounds_access() {
    let attributes = HtmlAttributeCollection::new();
    let _ = &attributes[0];
}

#[test]
fn writer_preserves_optional_attribute_values_and_order() {
    let mut html = HtmlWriter::new();
    html.write_empty_element_tag(HtmlTagId::Input.as_str())
        .unwrap();
    html.write_html_attribute(&HtmlAttribute::from_id(HtmlAttributeId::Disabled, None).unwrap())
        .unwrap();
    assert_eq!(html.state(), HtmlWriterState::Attribute);
    assert_eq!(html.as_str(), "<input disabled");
    html.write_html_attribute(&HtmlAttribute::new("ID", Some(String::new())).unwrap())
        .unwrap();
    assert_eq!(html.state(), HtmlWriterState::Tag);
    html.write_html_attribute(&HtmlAttribute::new("id", Some("a&\"b".into())).unwrap())
        .unwrap();
    html.write_html_attribute(&HtmlAttribute::new("data-Custom", None).unwrap())
        .unwrap();
    assert_eq!(html.state(), HtmlWriterState::Attribute);
    assert_eq!(
        html.finish(),
        "<input disabled ID=\"\" id=\"a&amp;&quot;b\" data-Custom/>"
    );
}

#[test]
fn writer_can_supply_a_value_after_a_valueless_attribute_object() {
    let mut html = HtmlWriter::new();
    html.write_start_tag(HtmlTagId::Div.as_str()).unwrap();
    html.write_attribute_name("hidden").unwrap();
    html.write_html_attribute(&HtmlAttribute::new("title", None).unwrap())
        .unwrap();
    html.write_attribute_value("a<b>").unwrap();
    html.write_text("text");
    html.write_end_tag(HtmlTagId::Div.as_str()).unwrap();
    assert_eq!(html.finish(), "<div hidden title=\"a&lt;b&gt;\">text</div>");
}

#[test]
fn writer_rejects_attribute_objects_outside_tags_without_mutation() {
    for value in [None, Some(String::new()), Some("value".into())] {
        let attribute = HtmlAttribute::new("title", value).unwrap();
        let mut html = HtmlWriter::new();
        for _ in 0..2 {
            let before = html.clone();
            assert_eq!(
                html.write_html_attribute(&attribute),
                Err(HtmlWriterError::AttributeOutsideTag)
            );
            assert_eq!(html, before);
            html.write_start_tag("p").unwrap();
            html.write_end_tag("p").unwrap();
        }
    }
}

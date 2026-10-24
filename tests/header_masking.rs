// Field-dispatch cases adapted from MimeKit's MimeAnonymizer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    Header, HeaderId, HeaderList,
    anonymize::{self, HeaderOptions},
};

fn assert_dispatch(fields: &[&str], value: &[u8], expected_value: &[u8]) {
    for field in fields {
        for spelling in [
            field.to_string(),
            field.to_ascii_lowercase(),
            field.to_ascii_uppercase(),
        ] {
            let prefix = format!("{spelling} \t:");
            let raw = [prefix.as_bytes(), value].concat();
            let record = Header::parse(&raw).unwrap();
            let before = record.clone();
            let expected = [prefix.as_bytes(), expected_value].concat();
            assert_eq!(anonymize::header(&record), expected, "field {spelling}");
            assert_eq!(
                anonymize::header_with_options(&record, &HeaderOptions::default()),
                expected,
                "default options, field {spelling}",
            );
            assert_eq!(expected.len(), record.raw().len());
            assert_eq!(record, before);
            assert_eq!(record.raw(), raw);
        }
    }
}

#[test]
fn source_address_and_identifier_fields_use_address_masking() {
    let value = b" \"A B\" <a@b.c> =?u?q?secret?=\r\n";
    let expected = anonymize::address_header_value(value);
    assert_dispatch(
        &[
            "Disposition-Notification-To",
            "Resent-Reply-To",
            "Resent-Sender",
            "Resent-From",
            "Resent-Bcc",
            "Resent-Cc",
            "Resent-To",
            "Reply-To",
            "Sender",
            "From",
            "Bcc",
            "Cc",
            "To",
            "Original-Message-Id",
            "Resent-Message-Id",
            "References",
            "In-Reply-To",
            "Message-Id",
            "Content-Id",
        ],
        value,
        &expected,
    );
    assert_eq!(expected, b" \"xxx\" <x@x.x> =?u?q?xxxxxx?=\r\n");
}

#[test]
fn received_and_content_fields_use_their_specific_helpers() {
    assert_dispatch(
        &["Received"],
        b" from host; 15 Nov 2017 14:16:18 -0000\r\n",
        b" from xxxx; 15 Nov 2017 14:16:18 -0000\r\n",
    );
    assert_dispatch(
        &["Content-Type"],
        b" text/plain; charset=UTF-8; name=secret.txt\r\n",
        b" text/plain; charset=UTF-8; name=xxxxxxxxxx\r\n",
    );
    assert_dispatch(
        &["Content-Disposition"],
        b" attachment; filename=\"A B\"; size=123\r\n",
        b" attachment; filename=\"xxx\"; size=xxx\r\n",
    );
    // Dispatch does not try to parse or validate either content-header value.
    assert_dispatch(
        &["Content-Type", "Content-Disposition"],
        b" invalid; charset=PRIVATE; name=secret\r\n",
        b" invalid; charset=PRIVATE; name=xxxxxx\r\n",
    );
}

#[test]
fn only_the_three_source_metadata_fields_pass_through_verbatim() {
    for value in [
        b"\r\n".as_slice(),
        b" invalid private data\r\n",
        b" \xff\0=?u?q?private?=\n",
    ] {
        assert_dispatch(
            &["Date", "MIME-Version", "Content-Transfer-Encoding"],
            value,
            value,
        );
    }

    // A standalone Header can contain arbitrary bytes and embedded header-like
    // lines in its value. Pass-through must not validate or silently truncate it.
    let mut value: Vec<_> = (0..=255u8).collect();
    value.extend_from_slice(b"\r\nSubject: private\r\n");
    assert_dispatch(
        &["Date", "MIME-Version", "Content-Transfer-Encoding"],
        &value,
        &value,
    );
}

#[test]
fn other_known_extension_and_empty_fields_use_unstructured_masking() {
    let value = b" \"A B\" <a@b.c> =?u?q?secret?=\r\n";
    let expected = anonymize::unstructured_header_value(value);
    assert_dispatch(
        &[
            "Subject",
            "Comments",
            "Resent-Date",
            "Date-Received",
            "Return-Path",
            "Return-Receipt-To",
            "Apparently-To",
            "Original-From",
            "Original-Recipient",
            "Original-Return-Address",
            "Content-Description",
            "Content-Location",
            "Content-Base",
            "Content-Md5",
            "Content-Length",
            "Content-Identifier",
            "Received-SPF",
            "List-Unsubscribe",
            "Autocrypt",
            "DomainKey-Signature",
            "X-Private",
            "X-To",
            "X-Date",
            "",
        ],
        value,
        &expected,
    );
    assert_eq!(expected, b" xx xx xxxxxxx =?u?q?xxxxxx?=\r\n");
}

#[test]
fn authentication_and_signature_fields_have_no_special_parser() {
    let value = b" v=1; d=example.com; b=abc?=;\r\n\t(private)\n";
    assert_dispatch(
        &[
            "ARC-Authentication-Results",
            "Authentication-Results",
            "ARC-Message-Signature",
            "ARC-Seal",
            "DKIM-Signature",
        ],
        value,
        &anonymize::unstructured_header_value(value),
    );
}

#[test]
fn malformed_records_are_wholly_byte_masked_instead_of_dispatched_or_dropped() {
    for raw in [
        b"No colon\n".as_slice(),
        b"Bad Name: =?u?q?private?=\n folded\n",
        b"Date\0: private\n",
        b"Bad\x7fName: value\n",
        b"\xff: value\r\n",
        b"\r\r\n",
        b" \torphan\n continuation\n",
        b"Broken without a final newline",
        b"Broken \x0b\x0c\\\"<>@=?\xff",
    ] {
        let headers = HeaderList::load(&mut &raw[..]).unwrap();
        assert_eq!(headers.len(), 1, "raw {raw:?}");
        let record = &headers.as_slice()[0];
        let before = record.clone();
        assert!(record.is_invalid());
        assert_eq!(record.id(), HeaderId::Unknown);
        assert_eq!(record.raw(), raw);
        let expected: Vec<_> = raw
            .iter()
            .map(|&byte| {
                if b" \t\r\n".contains(&byte) {
                    byte
                } else {
                    b'x'
                }
            })
            .collect();
        assert_eq!(anonymize::header(record), expected);
        assert_eq!(record, &before);
    }
}

#[test]
fn empty_field_names_remain_valid_and_keep_their_colon() {
    for (raw, expected) in [
        (b":".as_slice(), b":".as_slice()),
        (b": \t", b": \t"),
        (b": <a@b.c>\r\n", b": xxxxxxx\r\n"),
        (b":: private\n", b":x xxxxxxx\n"),
    ] {
        let headers = HeaderList::load(&mut &raw[..]).unwrap();
        let record = &headers.as_slice()[0];
        assert!(!record.is_invalid());
        assert_eq!(record.field(), Some(""));
        assert_eq!(record.id(), HeaderId::Unknown);
        assert_eq!(anonymize::header(record), expected);
    }
}

#[test]
fn folding_field_whitespace_bare_cr_and_eof_are_not_reformatted() {
    for (raw, expected) in [
        (
            b"To: Alice <a@b.c>".as_slice(),
            b"To: xxxxx <x@x.x>".as_slice(),
        ),
        (b"Subject: hi\r", b"Subject: xx\r"),
        (b"X-Opaque: \xff\0\rz", b"X-Opaque: xx\rx"),
        (
            b"tO \t: \"A \tB\"\r\n\t<a@b.c>\n",
            b"tO \t: \"xxxx\"\r\n\t<x@x.x>\n",
        ),
        (b"Content-Type:", b"Content-Type:"),
        // Newlines are not normalized, but the Received helper's escape rules
        // can replace a newline inside a comment with x.
        (b"Received: (a\\\r\n b)\n", b"Received: (x\\x\n x)\n"),
    ] {
        let headers = HeaderList::load(&mut &raw[..]).unwrap();
        assert_eq!(headers.len(), 1);
        let record = &headers.as_slice()[0];
        assert_eq!(anonymize::header(record), expected);
        assert_eq!(record.raw(), raw);
    }
}

#[test]
fn masking_records_preserves_order_duplicates_and_leaves_the_body_alone() {
    let raw = b"To: Alice <a@b.c>\nSubject: secret\nTo: Bob <b@c.d>\nBcc: Carol <c@d.e>\nBroken line\n\nprivate body";
    let mut reader = &raw[..];
    let headers = HeaderList::load(&mut reader).unwrap();
    let before = headers.clone();
    let output: Vec<_> = headers.iter().flat_map(anonymize::header).collect();
    assert_eq!(
        output,
        b"To: xxxxx <x@x.x>\nSubject: xxxxxx\nTo: xxx <x@x.x>\nBcc: xxxxx <x@x.x>\nxxxxxx xxxx\n",
    );
    assert_eq!(reader, b"private body");
    assert_eq!(headers, before);
    assert_eq!(headers.get_all("To").count(), 2);
    assert!(headers.get("MIME-Version").is_none());
}

#[test]
fn configured_fields_bypass_every_value_dispatch_class_without_validation() {
    for field in [
        "To",
        "References",
        "Content-Id",
        "Received",
        "Content-Type",
        "Content-Disposition",
        "Subject",
        "DKIM-Signature",
        "Authentication-Results",
        "X-Private",
        "",
    ] {
        let mut raw = format!("{field} \t:").into_bytes();
        raw.extend_from_slice(b" PRIVATE <name@example.com>; name=\"A B\"\0\xff\r\n\tmore\n");
        let record = Header::parse(&raw).unwrap();
        let before = record.clone();
        assert_ne!(anonymize::header(&record), raw, "field {field}");
        for spelling in [field.to_ascii_lowercase(), field.to_ascii_uppercase()] {
            let options = HeaderOptions {
                preserve_headers: vec![spelling],
            };
            let options_before = options.clone();
            assert_eq!(anonymize::header_with_options(&record, &options), raw);
            assert_eq!(options, options_before);
            assert_eq!(record, before);
        }
    }
}

#[test]
fn preservation_matches_exact_ascii_names_without_trimming_or_wildcards() {
    let record = Header::parse(b"sUbJeCt \t: private\n").unwrap();
    for name in [
        "Sub",
        "Subject:",
        " Subject",
        "Subject ",
        "Subject\t",
        "Subject\0",
        "*",
        "Subject*",
        "X-Subject",
        "\u{17f}ubject",
        "",
    ] {
        let options = HeaderOptions {
            preserve_headers: vec![name.into()],
        };
        assert_eq!(
            anonymize::header_with_options(&record, &options),
            anonymize::header(&record),
            "name {name:?}",
        );
        assert_eq!(options.preserve_headers, [name]);
    }
    let options = HeaderOptions {
        preserve_headers: vec!["SUBJECT".into()],
    };
    assert_eq!(
        anonymize::header_with_options(&record, &options),
        record.raw()
    );

    // A '*' can occur in a literal field name, but has no wildcard semantics.
    let literal = Header::parse(b"X-*: private\n").unwrap();
    let other = Header::parse(b"X-Other: private\n").unwrap();
    let options = HeaderOptions {
        preserve_headers: vec!["x-*".into()],
    };
    assert_eq!(
        anonymize::header_with_options(&literal, &options),
        literal.raw()
    );
    assert_eq!(
        anonymize::header_with_options(&other, &options),
        anonymize::header(&other)
    );

    // Extension fields share HeaderId::Unknown but not their preservation rule.
    let options = HeaderOptions {
        preserve_headers: vec!["Unknown".into()],
    };
    let unknown = Header::parse(b"Unknown: private\n").unwrap();
    let empty = Header::parse(b": private\n").unwrap();
    assert_eq!(unknown.id(), HeaderId::Unknown);
    assert_eq!(other.id(), HeaderId::Unknown);
    assert_eq!(
        anonymize::header_with_options(&unknown, &options),
        unknown.raw()
    );
    assert_eq!(
        anonymize::header_with_options(&other, &options),
        anonymize::header(&other)
    );
    assert_eq!(
        anonymize::header_with_options(&empty, &options),
        anonymize::header(&empty)
    );
}

#[test]
fn preserved_names_never_exempt_malformed_records() {
    let options = HeaderOptions {
        preserve_headers: vec![
            "".into(),
            "Date".into(),
            "Bad Name".into(),
            "Bad Name: secret\n".into(),
            "\u{e9}".into(),
            "Unknown".into(),
            "Broken".into(),
        ],
    };
    for raw in [
        b"Date\0: secret\n".as_slice(),
        b"Bad Name: secret\n",
        b"\xc3\xa9: secret\n",
        b"Broken",
        b" \torphan\n continuation\n",
    ] {
        let records = HeaderList::load(&mut &raw[..]).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records.as_slice()[0];
        assert!(record.is_invalid());
        let expected: Vec<_> = raw
            .iter()
            .map(|&byte| {
                if b" \t\r\n".contains(&byte) {
                    byte
                } else {
                    b'x'
                }
            })
            .collect();
        assert_eq!(anonymize::header_with_options(record, &options), expected);
        assert_eq!(record.raw(), raw);
    }
}

#[test]
fn preservation_lists_are_owned_and_edits_apply_on_the_next_call() {
    let subject = Header::parse(b"Subject: private\n").unwrap();
    let to = Header::parse(b"To: <private@example.com>\n").unwrap();
    let mut options = HeaderOptions::default();
    assert!(options.preserve_headers.is_empty());
    assert_eq!(
        anonymize::header_with_options(&subject, &options),
        anonymize::header(&subject)
    );
    options.preserve_headers.push("sUbJeCt".into());
    assert_eq!(
        anonymize::header_with_options(&subject, &options),
        subject.raw()
    );
    assert_eq!(
        anonymize::header_with_options(&to, &options),
        anonymize::header(&to)
    );

    options
        .preserve_headers
        .extend(["SUBJECT".into(), "To".into()]);
    assert_eq!(
        anonymize::header_with_options(&subject, &options),
        subject.raw()
    );
    assert_eq!(anonymize::header_with_options(&to, &options), to.raw());
    options.preserve_headers.reverse();
    assert_eq!(
        anonymize::header_with_options(&subject, &options),
        subject.raw()
    );
    assert_eq!(anonymize::header_with_options(&to, &options), to.raw());

    options
        .preserve_headers
        .retain(|name| !name.eq_ignore_ascii_case("subject"));
    assert_eq!(
        anonymize::header_with_options(&subject, &options),
        anonymize::header(&subject)
    );
    let mut copy = options.clone();
    copy.preserve_headers.clear();
    assert_eq!(
        anonymize::header_with_options(&to, &copy),
        anonymize::header(&to)
    );
    assert_eq!(anonymize::header_with_options(&to, &options), to.raw());
    options.preserve_headers.clear();
    assert_eq!(options, HeaderOptions::default());
}

#[test]
fn preservation_copies_opaque_values_and_does_not_change_raw_value_helpers() {
    let mut raw = b"Subject:\r\nX-Private: ".to_vec();
    raw.extend(0..=255u8);
    raw.extend_from_slice(b"\r\n\r\nprivate body-like bytes\r\n");
    let record = Header::parse(&raw).unwrap();
    let options = HeaderOptions {
        preserve_headers: vec!["Subject".into()],
    };
    assert_eq!(anonymize::header_with_options(&record, &options), raw);
    assert_eq!(record.raw(), raw);
    let default_output = [
        b"Subject:".as_slice(),
        &anonymize::unstructured_header_value(record.raw_value()),
    ]
    .concat();
    assert_eq!(anonymize::header(&record), default_output);
    assert_ne!(default_output, raw);
}

#[test]
fn additional_preservation_does_not_disable_builtin_pass_through() {
    for field in ["Date", "MIME-Version", "Content-Transfer-Encoding"] {
        let raw = format!("{field}: invalid private data\n");
        let record = Header::parse(raw.as_bytes()).unwrap();
        for fields in [vec![], vec!["Subject".into()], vec![field.into()]] {
            let options = HeaderOptions {
                preserve_headers: fields,
            };
            assert_eq!(
                anonymize::header_with_options(&record, &options),
                record.raw()
            );
        }
    }
}

#[test]
fn preservation_applies_to_each_explicit_record_without_traversing_the_body() {
    let raw =
        b"Subject: first\nTo: Alice <a@b.c>\nSubject: second\nX-Private: keep\n\nprivate body";
    let mut reader = &raw[..];
    let headers = HeaderList::load(&mut reader).unwrap();
    let before = headers.clone();
    let options = HeaderOptions {
        preserve_headers: vec!["subject".into(), "X-PRIVATE".into()],
    };
    let options_before = options.clone();
    let output: Vec<_> = headers
        .iter()
        .flat_map(|record| anonymize::header_with_options(record, &options))
        .collect();
    assert_eq!(
        output,
        b"Subject: first\nTo: xxxxx <x@x.x>\nSubject: second\nX-Private: keep\n",
    );
    assert_eq!(headers, before);
    assert_eq!(options, options_before);
    assert_eq!(reader, b"private body");
}

// Field-dispatch cases adapted from MimeKit's MimeAnonymizer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{Header, HeaderId, HeaderList, anonymize};

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

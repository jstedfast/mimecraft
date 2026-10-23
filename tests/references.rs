// Includes cases adapted from MimeKit's UnitTests/Utils/MimeUtilsTests.cs.
// Scanning and recovery follow MimeKit's Utils/MimeUtils.cs and Utils/ParseUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::iter::FusedIterator;

use mimecraft::{
    HeaderId, HeaderParser, MessageId, MessageIdParseError, MessageIdParseErrorKind as Kind,
    References,
};

fn collect_ids(input: &[u8]) -> Result<Vec<String>, MessageIdParseError> {
    References::new(input)
        .map(|result| result.map(MessageId::into_string))
        .collect()
}

#[test]
fn source_good_reference_vectors() {
    for (input, expected) in [
        ("<local-part@domain1@domain2>", "local-part@domain1@domain2"),
        ("<local-part@>", "local-part@"),
        ("<local-part>", "local-part"),
        (
            "<:invalid-local-part;@domain.com>",
            ":invalid-local-part;@domain.com",
        ),
    ] {
        assert_eq!(collect_ids(input.as_bytes()).unwrap(), [expected]);
    }
}

#[test]
fn source_broken_references_report_errors_instead_of_disappearing() {
    for input in [
        " (this is an unterminated comment...",
        "<",
        "<local-part",
        "<local-part;",
        "<local-part@ (unterminated comment...",
        "<local-part@",
        "<local-part @ bad-domain (comment) . (comment com",
        "<local-part@[127.0",
    ] {
        let mut references = References::new(input.as_bytes());
        assert!(references.next().unwrap().is_err(), "{input:?}");
        assert!(references.next().is_none(), "{input:?}");
    }
    assert!(
        References::new(b"(this is just a comment)")
            .next()
            .is_none()
    );
}

#[test]
fn order_duplicates_and_adjacent_identifiers_are_preserved() {
    let input = b" <a@b><c@d>,<a@b>;\r\n\t<local@> <[image@@example.com]>";
    assert_eq!(
        collect_ids(input).unwrap(),
        ["a@b", "c@d", "a@b", "local@", "[image@example.com]"]
    );
}

#[test]
fn only_bracketed_candidates_outside_comments_and_quoted_prose_are_parsed() {
    for input in [
        &b""[..],
        b" \t\r\n",
        b"bare@example.com local-id",
        b")@,;.:>][\\",
        b"(outer (inner <hidden@b>) done)",
        b"\"<hidden@b>\"",
        b"\xff\xfe bare@example.com \"\xff <hidden@b>\"",
    ] {
        assert!(References::new(input).next().is_none(), "{input:?}");
    }
    let input = br#"Re: bare@x "(quoted <hidden@x>)" (outer (<hidden@y>)) <visible@z>"#;
    assert_eq!(collect_ids(input).unwrap(), ["visible@z"]);
    assert_eq!(
        collect_ids(b"atom\"<hidden@b>\"<visible@c>").unwrap(),
        ["visible@c"]
    );
}

#[test]
fn escaped_quotes_and_comment_parentheses_do_not_expose_hidden_candidates() {
    let input = br#""escaped \" <hidden@b>" (escaped \) <hidden@c>) <visible@d>"#;
    assert_eq!(collect_ids(input).unwrap(), ["visible@d"]);
    for count in 0..10 {
        let input = format!("\"quoted {}\" <visible@d>", "\\".repeat(count));
        if count % 2 == 0 {
            assert_eq!(collect_ids(input.as_bytes()).unwrap(), ["visible@d"]);
        } else {
            assert_eq!(
                collect_ids(input.as_bytes()).unwrap_err().kind(),
                Kind::IncompleteQuotedString
            );
        }
    }
}

#[test]
fn candidate_normalization_matches_message_id_parsing() {
    for input in [
        b"<\"local > part\"@domain>".as_slice(),
        b"<\"x\r\n y\"@example.com>",
        b"<[image@@example.com]>",
        b"<a@b@c>",
        b"<a@(comment)example . test.>",
        b"<a@[ \tIPv6:2001:db8::1 \r\n]>",
        b"<a(comment)@b>",
        "<\u{e9}@\u{6d4b}\u{8bd5}.cn>".as_bytes(),
    ] {
        assert_eq!(
            References::new(input)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            [MessageId::parse(input).unwrap()]
        );
    }
}

#[test]
fn errors_are_recoverable_and_offsets_cover_the_whole_input() {
    let input = b"prose <> <one@b> <two@.bad> <three@b>";
    let mut references = References::new(input);
    let error = references.next().unwrap().unwrap_err();
    assert_eq!(error.kind(), Kind::EmptyId);
    assert_eq!(error.offset(), 6);
    assert_eq!(references.next().unwrap().unwrap().as_str(), "one@b");
    let error = references.next().unwrap().unwrap_err();
    assert_eq!(error.kind(), Kind::InvalidDomain);
    assert_eq!(error.offset(), 22);
    assert_eq!(references.next().unwrap().unwrap().as_str(), "three@b");
    assert!(references.next().is_none());
}

#[test]
fn recovery_uses_the_failure_cursor_not_the_next_closing_bracket() {
    for (input, kind, offset, expected) in [
        ("<a@<b@c>", Kind::InvalidDomain, 3, "b@c"),
        ("<a@b,<c@d>", Kind::IncompleteId, 4, "c@d"),
        ("<a@[x\\<inside@b>]>", Kind::InvalidDomain, 5, "inside@b"),
        ("<a@b@@c> <next@d>", Kind::InvalidDomain, 5, "next@d"),
    ] {
        let mut references = References::new(input.as_bytes());
        let error = references.next().unwrap().unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.offset(), offset, "{input:?}");
        assert_eq!(references.next().unwrap().unwrap().as_str(), expected);
        assert!(references.next().is_none(), "{input:?}");
    }
}

#[test]
fn invalid_utf8_recovery_does_not_restart_at_the_diagnostic_offset() {
    // The malformed local part already consumed the embedded '<'. Restarting
    // at the UTF-8 diagnostic would incorrectly discover hidden@x.
    let input = b"<bad\xff<hidden@x> <good@y>";
    let mut references = References::new(input);
    let error = references.next().unwrap().unwrap_err();
    assert_eq!(error.kind(), Kind::InvalidUtf8);
    assert_eq!(error.offset(), 4);
    assert_eq!(references.next().unwrap().unwrap().as_str(), "good@y");
    assert!(references.next().is_none());

    let mut references = References::new(b"<a@\xff. <b@c>");
    assert_eq!(references.next().unwrap().unwrap_err().offset(), 3);
    assert_eq!(references.next().unwrap().unwrap().as_str(), "b@c");
    assert!(references.next().is_none());
}

#[test]
fn unterminated_constructs_consume_the_remainder_and_report_once() {
    for (input, kind, offset) in [
        ("(unterminated <hidden@b>", Kind::IncompleteComment, 0),
        ("\"unterminated <hidden@b>", Kind::IncompleteQuotedString, 0),
        (
            "<\"unterminated <hidden@b>",
            Kind::IncompleteQuotedString,
            1,
        ),
        ("<a@(unterminated <hidden@b>", Kind::IncompleteComment, 3),
        ("<a@[unfinished", Kind::IncompleteDomainLiteral, 3),
        ("<unfinished", Kind::IncompleteId, 11),
    ] {
        let mut references = References::new(input.as_bytes());
        let error = references.next().unwrap().unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.offset(), offset, "{input:?}");
        for _ in 0..3 {
            assert!(references.next().is_none(), "{input:?}");
        }
    }
}

#[test]
fn offsets_are_bytes_and_relative_to_the_supplied_slice() {
    let input = "\u{e9} <a@b> <> <c@.d> <last@x>";
    for start in [0, input.find("<a@b>").unwrap()] {
        let mut references = References::new(&input.as_bytes()[start..]);
        assert_eq!(references.next().unwrap().unwrap().as_str(), "a@b");
        let error = references.next().unwrap().unwrap_err();
        assert_eq!(error.kind(), Kind::EmptyId);
        assert_eq!(error.offset(), input.find("<>").unwrap() - start);
        let error = references.next().unwrap().unwrap_err();
        assert_eq!(error.kind(), Kind::InvalidDomain);
        assert_eq!(error.offset(), input.find(".d").unwrap() - start);
        assert_eq!(references.next().unwrap().unwrap().as_str(), "last@x");
        assert!(references.next().is_none());
    }
}

#[test]
fn iteration_is_lazy_cloneable_and_fused() {
    fn assert_fused<I: FusedIterator>(_: &I) {}

    let mut references = References::new(b"<first@b> <> <last@b> (unterminated");
    assert_fused(&references);
    assert_eq!(references.next().unwrap().unwrap().as_str(), "first@b");
    let before_error = references.clone();
    assert_eq!(
        references.next().unwrap().unwrap_err().kind(),
        Kind::EmptyId
    );
    assert_eq!(
        before_error.skip(1).collect::<Vec<_>>(),
        references.clone().collect::<Vec<_>>()
    );
    assert_eq!(
        references
            .by_ref()
            .take(1)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()[0]
            .as_str(),
        "last@b"
    );
    assert_eq!(
        references.next().unwrap().unwrap_err().kind(),
        Kind::IncompleteComment
    );
    assert!(references.next().is_none());
    assert!(references.clone().next().is_none());
    assert!(references.next().is_none());
}

#[test]
fn collection_stops_on_error_but_manual_iteration_can_resume() {
    let mut references = References::new(b"<one@b> <> <two@b>");
    let error = references
        .by_ref()
        .collect::<Result<Vec<_>, _>>()
        .unwrap_err();
    assert_eq!(error.kind(), Kind::EmptyId);
    assert_eq!(references.next().unwrap().unwrap().as_str(), "two@b");
    assert!(references.next().is_none());
}

#[test]
fn deeply_nested_comments_and_long_lists_preserve_order() {
    let mut input = format!("{}<hidden@b>{}", "(".repeat(20_000), ")".repeat(20_000));
    let mut expected = Vec::new();
    for index in 0..1_000 {
        let token = format!("id-{index}@example.test");
        input.push_str(&format!(" \"<hidden@b>\" (<hidden@c>) bare@d <{token}>"));
        expected.push(token);
    }
    assert_eq!(collect_ids(input.as_bytes()).unwrap(), expected);
}

#[test]
fn raw_references_and_in_reply_to_values_remain_unchanged() {
    let input =
        b"References: <one@x>\r\n\t<>\r\n\t<two@x>\r\nIn-Reply-To: prose <parent@x>\r\n\r\nbody";
    for chunk_size in 1..=input.len() {
        let mut parser = HeaderParser::new();
        let mut headers = Vec::new();
        let mut consumed = 0;
        for chunk in input.chunks(chunk_size) {
            consumed += parser.parse(chunk, &mut headers).unwrap();
            if parser.is_finished() {
                break;
            }
        }
        assert_eq!(&input[consumed..], b"body");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].id(), HeaderId::References);
        assert_eq!(headers[1].id(), HeaderId::InReplyTo);
        let original = headers[0].raw().to_vec();
        let entries: Vec<_> = References::new(headers[0].raw_value()).collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].as_ref().unwrap().as_str(), "one@x");
        assert_eq!(entries[1].as_ref().unwrap_err().kind(), Kind::EmptyId);
        assert_eq!(entries[1].as_ref().unwrap_err().offset(), 11);
        assert_eq!(entries[2].as_ref().unwrap().as_str(), "two@x");
        assert_eq!(headers[0].raw(), original);
        assert_eq!(collect_ids(headers[1].raw_value()).unwrap(), ["parent@x"]);
    }
}

fn check_iteration_bounds(input: &[u8]) {
    let mut references = References::new(input);
    let mut count = 0;
    for result in references.by_ref() {
        count += 1;
        assert!(count <= input.len(), "iterator made no progress: {input:?}");
        match result {
            Ok(id) => {
                assert!(!id.as_str().is_empty());
                assert!(id.as_str().len() <= input.len());
            }
            Err(error) => assert!(error.offset() <= input.len(), "{input:?}"),
        }
    }
    assert!(references.next().is_none());
    assert!(references.next().is_none());
}

#[test]
fn arbitrary_bytes_and_all_truncations_terminate_with_bounded_errors() {
    for input in [
        b"(comment) <a@b> \"<hidden@c>\" <> <x@.y> <last@d>".as_slice(),
        b"<bad\xff<hidden@x> <good@y>",
        b"<a@[x\\<inside@b>]> (unterminated",
        "<\u{e9}@\u{6d4b}\u{8bd5}.cn> <>".as_bytes(),
    ] {
        for end in 0..=input.len() {
            check_iteration_bounds(&input[..end]);
        }
    }
    let alphabet = b"ab01.@<>[]()\"\\ \t\r\n";
    let mut state = 0x1234_5678u32;
    for sample in 0..20_000 {
        let mut input = Vec::new();
        for _ in 0..sample % 64 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let byte = (state >> 24) as u8;
            input.push(if sample % 2 == 0 {
                alphabet[usize::from(byte) % alphabet.len()]
            } else {
                byte
            });
        }
        check_iteration_bounds(&input);
    }
}

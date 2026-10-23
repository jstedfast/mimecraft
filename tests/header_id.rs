// Includes cases adapted from MimeKit's HeaderTests.cs and HeaderId.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{HeaderId, HeaderParser};

#[test]
fn source_canonical_spellings_and_rust_acronym_casing() {
    let cases = [
        (
            HeaderId::ArcAuthenticationResults,
            "ARC-Authentication-Results",
        ),
        (HeaderId::ArcMessageSignature, "ARC-Message-Signature"),
        (HeaderId::ArcSeal, "ARC-Seal"),
        (HeaderId::AutoSubmitted, "Auto-Submitted"),
        (HeaderId::Autosubmitted, "Autosubmitted"),
        (HeaderId::ContentId, "Content-Id"),
        (HeaderId::ContentMd5, "Content-Md5"),
        (HeaderId::ContentType, "Content-Type"),
        (HeaderId::DkimSignature, "DKIM-Signature"),
        (HeaderId::DomainKeySignature, "DomainKey-Signature"),
        (HeaderId::InReplyTo, "In-Reply-To"),
        (HeaderId::ListId, "List-Id"),
        (HeaderId::MessageId, "Message-Id"),
        (HeaderId::MimeVersion, "MIME-Version"),
        (HeaderId::NntpPostingHost, "Nntp-Posting-Host"),
        (
            HeaderId::PreventNonDeliveryReport,
            "Prevent-NonDelivery-Report",
        ),
        (HeaderId::ReceivedSpf, "Received-SPF"),
        (HeaderId::ResentMessageId, "Resent-Message-Id"),
        (HeaderId::TlsRequired, "TLS-Required"),
        (HeaderId::X400MtsIdentifier, "X400-MTS-Identifier"),
        (HeaderId::XMsMailPriority, "X-MSMail-Priority"),
    ];
    for (id, name) in cases {
        assert_eq!(id.as_str(), name);
        assert_eq!(id.to_string(), name);
        assert_eq!(HeaderId::from_name(name), id);
        assert_eq!(HeaderId::from_name(&name.to_ascii_uppercase()), id);
        assert_eq!(HeaderId::from_name(&name.to_ascii_lowercase()), id);
    }
    assert_ne!(HeaderId::AutoSubmitted, HeaderId::Autosubmitted);
}

#[test]
fn unknown_is_explicit_and_does_not_normalize_invalid_names() {
    const UNKNOWN_LABEL: &str = HeaderId::Unknown.as_str();
    assert_eq!(UNKNOWN_LABEL, "Unknown");
    assert_eq!(HeaderId::default(), HeaderId::Unknown);
    for name in [
        "",
        "Unknown",
        "X-MadeUp-Header",
        "!",
        "zzzz",
        "Subject:",
        "Subject: value",
        " Subject",
        "Subject ",
        "Subject\t",
        "Subject\r\n",
        "Subject\0",
        "ContentType",
        "MimeVersion",
        "D\u{212a}IM-Signature",
        "Subj\u{00e9}ct",
        "\u{00a0}Subject",
    ] {
        assert_eq!(HeaderId::from_name(name), HeaderId::Unknown, "{name:?}");
    }
}

#[test]
fn identifiers_preserve_raw_records_and_distinguish_unknown_from_invalid() {
    let input = b"sUbJeCt \t: first\r\n\tfolded\n\
X-Custom: untouched\n\
Unknown: also custom\n\
: empty name\n\
Bad Name: invalid\n\
Subject \0: invalid\n\
\xff: invalid\n\
SUBJECT: duplicate\r\n\r\nbody";
    let expected = [
        (HeaderId::Subject, Some("sUbJeCt"), false),
        (HeaderId::Unknown, Some("X-Custom"), false),
        (HeaderId::Unknown, Some("Unknown"), false),
        (HeaderId::Unknown, Some(""), false),
        (HeaderId::Unknown, None, true),
        (HeaderId::Unknown, None, true),
        (HeaderId::Unknown, None, true),
        (HeaderId::Subject, Some("SUBJECT"), false),
    ];
    for split in 0..=input.len() {
        let mut parser = HeaderParser::new();
        let mut headers = Vec::new();
        let first = parser.parse(&input[..split], &mut headers).unwrap();
        let second = parser.parse(&input[split..], &mut headers).unwrap();
        parser.finish(&mut headers).unwrap();
        assert_eq!(&input[first + second..], b"body");
        assert_eq!(headers.len(), expected.len());
        for (header, &(id, field, invalid)) in headers.iter().zip(&expected) {
            assert_eq!(header.id(), id);
            assert_eq!(header.field(), field);
            assert_eq!(header.is_invalid(), invalid);
        }
        let mut restored = Vec::new();
        for header in &headers {
            restored.extend_from_slice(header.raw());
        }
        restored.extend_from_slice(parser.body_separator().unwrap());
        assert_eq!(restored, input[..first + second]);
        let subjects: Vec<_> = headers
            .iter()
            .filter(|header| header.id() == HeaderId::Subject)
            .map(|header| header.raw_value())
            .collect();
        assert_eq!(
            subjects,
            [b" first\r\n\tfolded\n".as_slice(), b" duplicate\r\n"]
        );
    }
}

#[test]
fn cloned_and_reset_parsers_do_not_share_header_identifiers() {
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    parser.parse(b"Content-", &mut headers).unwrap();
    let mut cloned = parser.clone();

    parser.parse(b"Type: text/plain", &mut headers).unwrap();
    parser.finish(&mut headers).unwrap();
    assert_eq!(headers[0].id(), HeaderId::ContentType);

    cloned.parse(b"Id: <id>\n\n", &mut headers).unwrap();
    assert_eq!(headers[1].id(), HeaderId::ContentId);
    let saved = headers[1].clone();

    parser.reset();
    parser.parse(b"X-Custom: value\n\n", &mut headers).unwrap();
    assert_eq!(headers[2].id(), HeaderId::Unknown);
    assert_eq!(headers[0].id(), HeaderId::ContentType);
    assert_eq!(saved.id(), HeaderId::ContentId);
    assert_eq!(saved.raw(), b"Content-Id: <id>\n");
}

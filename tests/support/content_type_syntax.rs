// Adapted from MimeKit's MimeParserTests.ContentTypeSyntaxCases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

// Regression cases inspired by https://github.com/noxxi/mime-is-broken.
pub const CASES: &[(&str, bool, &str, Option<&str>)] = &[
    ("text/plain junk", true, "text/plain", None),
    ("text/plain junk; charset=utf-8", true, "text/plain", None),
    ("text/plain charset=utf-8", true, "text/plain", None),
    (
        "text/plain (comment; charset=ignored) junk",
        true,
        "text/plain",
        None,
    ),
    ("text/plain\r\n\tjunk", true, "text/plain", None),
    (
        "\r\n\ttext/plain (comment)\r\n\tjunk; charset=utf-8",
        true,
        "text/plain",
        None,
    ),
    ("text/plain, text/html", true, "text/plain", None),
    ("text/plain\"junk\"", true, "text/plain", None),
    ("text/plain", false, "text/plain", None),
    ("text/plain (comment; junk)", false, "text/plain", None),
    (
        "text/plain (comment); charset=utf-8",
        false,
        "text/plain",
        Some("utf-8"),
    ),
    (
        "text/plain;\r\n\tcharset=\"utf-8\"",
        false,
        "text/plain",
        Some("utf-8"),
    ),
    ("text/plain;", false, "text/plain", None),
    ("text/plain; charset", false, "text/plain", None),
    (
        "text/plain; charset=utf-8; broken",
        false,
        "text/plain",
        None,
    ),
    ("text/plain; (unterminated", false, "text/plain", None),
    (
        "text/plain (unterminated",
        true,
        "application/octet-stream",
        None,
    ),
    (
        "garbage; charset=utf-8",
        true,
        "application/octet-stream",
        Some("utf-8"),
    ),
];

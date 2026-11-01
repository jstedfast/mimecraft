// Adapted from MimeKit's MimeComplianceEncodedWordTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

// Regression cases inspired by https://github.com/noxxi/mime-is-broken.
pub const SYNTAX: &[(&str, usize)] = &[
    ("ordinary text", 0),
    ("=?utf-8?Q?caf=C3=A9?=", 0),
    ("=?UTF-8?q?hello_world?=", 0),
    ("=?utf-8?B?YQ==?=", 0),
    ("=?utf-8?b?YWI=?=", 0),
    ("=?utf-8?B?YWJj?=", 0),
    ("=?x-unknown*en?Q?hello?=", 0),
    ("=?utf-8*?Q?hello?=", 0),
    ("=?utf-8?Q?first?=\r\n\t=?utf-8?Q?second?=", 0),
    ("=?utf-8?Q?()\\:.,?=", 0),
    ("=?utf-8?Q?=ff=00?=", 0),
    ("=?", 1),
    ("=?utf-8?Q?unfinished", 1),
    ("=??Q?hello?=", 1),
    ("=?*en?Q?hello?=", 1),
    ("=?utf.8?Q?hello?=", 1),
    ("=?utf 8?Q?hello?=", 1),
    ("=?utf-8?X?hello?=", 1),
    ("=?utf-8?QQ?hello?=", 1),
    ("=?utf-8??hello?=", 1),
    ("=?utf-8?Q??=", 1),
    ("=?utf-8?Q?a b?=", 1),
    ("=?utf-8?Q?a\tb?=", 1),
    ("=?utf-8?Q?a\r\n b?=", 1),
    ("=?utf-8?Q?caf\u{e9}?=", 1),
    ("=?utf-8?Q?bad=GG?=", 1),
    ("=?utf-8?Q?bad=A?=", 1),
    ("=?utf-8?Q?bad=?=", 1),
    ("=?utf-8?B?YQ?=", 1),
    ("=?utf-8?B?YQ=?=", 1),
    ("=?utf-8?B?Y===?=", 1),
    ("=?utf-8?B?=WJj?=", 1),
    ("=?utf-8?B?YWJj!?=", 1),
    ("=?utf-8?B?YQ==YWJj?=", 1),
    ("prefix=?utf-8?Q?hello?=", 1),
    ("=?utf-8?Q?hello?=suffix", 1),
    ("=?utf-8?Q?first?==?utf-8?Q?second?=", 2),
    ("=?broken=?utf-8?Q?hello?=", 2),
    ("=?utf-8?Q?broken=?utf-8?Q?hello?=", 2),
    ("(=?utf-8?Q?hello?=)", 1),
];

pub const ADDRESSES: &[(&str, usize, usize)] = &[
    ("=?utf-8?Q?hello_world?= <a@example.com>", 0, 0),
    ("=?utf-8?B?YQ==?= <a@example.com>", 0, 0),
    ("=?utf-8?Q?group?= : a@example.com;", 0, 0),
    ("a@example.com (=?utf-8?Q?hello?=)", 0, 0),
    ("(outer (=?utf-8?Q?hello?=)) a@example.com", 0, 0),
    ("a (=?utf-8?Q?hello?=) @ example.com", 0, 0),
    (
        "=?utf-8?Q?hello?= (=?utf-8?Q?comment?=) <a@example.com>",
        0,
        0,
    ),
    ("=?utf-8?Q?bad=GG?= <a@example.com>", 1, 0),
    ("=?utf-8?B?YQ?= <a@example.com>", 1, 0),
    ("=?utf-8?X?hello?= <a@example.com>", 1, 0),
    ("=?utf-8?Q?hello.world?= <a@example.com>", 1, 0),
    ("=?utf-8?Q?hello=2Eworld?= <a@example.com>", 0, 0),
    ("=?utf-8?Q?group.name?= : a@example.com;", 1, 0),
    ("=?utf-8?Q?hello?=<a@example.com>", 1, 0),
    ("=?utf-8?Q?group?=: a@example.com;", 1, 0),
    ("prefix=?utf-8?Q?hello?= <a@example.com>", 1, 0),
    ("a@example.com (=?utf-8?Q?hello.world?=)", 0, 0),
    ("a@example.com (=?utf-8?Q?hello(world)?=)", 1, 0),
    ("a@example.com (=?utf-8?Q?hello\\world?=)", 1, 0),
    ("a@example.com (prefix=?utf-8?Q?hello?=)", 1, 0),
    ("a@example.com (=?utf-8?Q?bad=GG?=)", 1, 0),
    ("\"=?utf-8?Q?hello?=\" <a@example.com>", 0, 1),
    ("\"=?utf-8?Q?bad=GG?=\" <a@example.com>", 1, 1),
    ("\"=?utf-8?Q?bad=?=\" <a@example.com>", 1, 1),
    ("\"=?utf-8?Q?group?=\" : a@example.com;", 0, 1),
    ("=?utf-8?Q?user?=@example.com", 0, 1),
    ("\"=?utf-8?Q?user?=\"@example.com", 0, 1),
    ("a@=?utf-8?Q?example?=.com", 0, 1),
    ("a@=?utf-8?Q?example.com?=", 0, 1),
    ("a@[=?utf-8?Q?example?=]", 0, 1),
    ("Name <a@=?utf-8?Q?example?=.com>", 0, 1),
    ("=?literal@example.com", 0, 0),
    ("\"=?literal\"@example.com", 0, 0),
    ("a@example.com, =?utf-8?Q?bad=GG?= <b@example.com>", 1, 0),
    (
        "=?utf-8?Q?group?= : =?utf-8?Q?bad=GG?= <a@example.com>;",
        1,
        0,
    ),
    (
        "=?utf-8?Q?first?= <a@example.com>, \"=?utf-8?Q?second?=\" <b@example.com>",
        0,
        1,
    ),
    (
        "(=?utf-8?Q?bad=GG?=) \"=?utf-8?Q?hello?=\" <a@example.com>",
        1,
        1,
    ),
];

// Recovery may re-enter a quote/comment that an earlier domain-literal scan hid.
pub const RECOVERY: &[(&str, usize, usize)] = &[
    ("<[\"]=?<", 0, 0),
    (">](\")=?:", 0, 0),
    ("[\"],=?:", 0, 0),
    ("[\"]=?[\"<", 1, 0),
    ("<[\"]=?:", 0, 0),
    ("\0;(\")=?:", 0, 0),
    ("[\"];=?:", 0, 0),
    ("=;[\"]=?<", 0, 0),
    ("[\"],:=?<", 0, 0),
    ("<[=(]=?<", 0, 0),
    ("<[.(]=?<", 0, 0),
    ("[\"]=?\":\"", 1, 0),
    ("=;(\")=?<", 0, 0),
    ("[\"]=?,\"<", 1, 0),
    ("[(@],=?<", 0, 0),
    ("<[,(]=?<", 0, 0),
    (">[\"]=?:", 0, 0),
    ("[\"],=?<", 1, 0),
    ("Name <a =?utf-8?Q?hello?= <b@example.com>", 0, 0),
    ("Name <a\r=?utf-8?Q?hello?= <b@example.com>", 0, 1),
    ("Name <a\r\n =?utf-8?Q?hello?= <b@example.com>", 0, 1),
    ("Name <a@b =?utf-8?Q?bad=GG?= <b@example.com>", 1, 0),
    ("Name <a@b\r=?utf-8?Q?bad=GG?= <b@example.com>", 1, 1),
    ("Name <a@b\r\n =?utf-8?Q?bad=GG?= <b@example.com>", 1, 1),
];

pub fn headers() -> Vec<(String, usize, usize)> {
    let mut headers = Vec::new();
    for field in ["Subject", "Comments", "Content-Description", "X-Example"] {
        for &(value, count) in SYNTAX {
            headers.push((format!("{field}: {value}"), count, 0));
        }
        for (length, count) in [(63, 0), (64, 1)] {
            let word = format!("=?utf-8?Q?{}?=", "a".repeat(length));
            assert_eq!(word.len(), length + 12);
            headers.push((format!("{field}: {word}"), count, 0));
        }
    }
    for &(value, invalid, placement) in ADDRESSES.iter().chain(RECOVERY) {
        headers.push((format!("To: {value}"), invalid, placement));
    }
    for field in [
        "From",
        "Sender",
        "Reply-To",
        "Cc",
        "Bcc",
        "Resent-From",
        "Resent-Sender",
        "Resent-Reply-To",
        "Resent-To",
        "Resent-Cc",
        "Resent-Bcc",
        "Disposition-Notification-To",
    ] {
        headers.push((format!("{field}: =?utf-8?Q?bad=GG?= <a@example.com>"), 1, 0));
    }
    for field in [
        "Date",
        "Message-Id",
        "Received",
        "Content-Type",
        "Content-Disposition",
        "Return-Path",
    ] {
        headers.push((format!("{field}: =?utf-8?Q?bad=GG?="), 0, 0));
    }
    headers
}

pub const PRESERVED: &str = "Subject: =?utf-8?Q?bad=GG?=\r\n\
To: =?x-unknown?Q?hello?= <a@example.com>, \"=?utf-8?Q?other?=\" <b@example.com>\r\n\
To: =?utf-8?Q?ignored=GG?= <ignored@example.com>\r\n\r\nbody\r\n";

pub const POSITIONS: &str = "X-Prefix: \u{e9}\r\nTo: (\u{e9}) =?utf-8?Q?bad=GG?= <a@example.com>,\r\n\
\t\"=?utf-8?Q?other?=\" <b@example.com>\r\nSubject: =?utf-8?Q?bad=G?=\r\n\r\nbody\r\n";

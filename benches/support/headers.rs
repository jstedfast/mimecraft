// Adapted from MimeKit's Benchmarks/AddressParserBenchmarks.cs and
// Benchmarks/Utils/Rfc2047Benchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{Address, AddressList, rfc2047};
use std::hint::black_box;

use crate::fingerprint::fingerprint;

pub const CASES: [&str; 41] = [
    "Single",
    "Bare",
    "Recipients",
    "Utf8",
    "Encoded",
    "MixedEncoded",
    "Groups",
    "Broken",
    "LongAtoms",
    "Quoted",
    "Comment",
    "UnicodeAddr",
    "QuotedLocal",
    "CFWSDomain",
    "TrailingDot",
    "Utf8Cjk",
    "UnknownCharset",
    "QuotedUtf8",
    "QuotedEncoded",
    "Utf8Emoji",
    "EncodedCjk",
    "InvalidEncoded",
    "LongName",
    "LongUtf8",
    "LongEquals",
    "CommentNames",
    "CommentTabs",
    "CommentUtf8",
    "CommentEncoded",
    "LongComment",
    "LongCommentUtf8",
    "CommentUtf8Tabs",
    "CommentEncodedTabs",
    "CommentUnknownCharsetTabs",
    "LongCommentUtf8Tabs",
    "LongCommentEqualsTabs",
    "EscapedName",
    "FoldedName",
    "LongQuotedName",
    "LongQuotedUtf8",
    "LongQuotedEquals",
];

pub const RFC2047_INPUT: &[u8] = b"I'm so happy! =?utf-8?b?8J+YgA==?= I love MIME so much =?utf-8?b?4p2k77iP4oCN8J+UpSE=?= Isn't it great?";

pub struct Workload {
    pub name: &'static str,
    pub text: String,
}

impl Workload {
    pub fn check(&self) {
        let (_, input, output) = REFERENCES.iter().find(|r| r.0 == self.name).unwrap();
        assert_eq!(
            fingerprint(self.text.as_bytes()),
            *input,
            "{} input",
            self.name
        );
        let bytes = parse_bytes(self);
        assert_eq!(
            address_fingerprint(&bytes),
            *output,
            "{} parsed names/addresses",
            self.name
        );
        assert_eq!(parse_string(self), bytes);
    }
}

// FNV-1a references from MimeKit's original generators and parser. The output
// fingerprint includes every name/address and group boundary, not just counts.
type Reference = (&'static str, (usize, u64), (usize, u64));
const REFERENCES: [Reference; 41] = [
    ("Single", (29, 0x468c83dd95ff8a97), (29, 0x001a0ddfaef08064)),
    ("Bare", (22, 0x442e129fd3810c3c), (25, 0xd894aa6010d9298f)),
    (
        "Recipients",
        (2678, 0x35aa165f3bc5d917),
        (2480, 0x9e0821d1bbd1d6ab),
    ),
    (
        "Utf8",
        (3678, 0xac744bc4bfc5fbaf),
        (3480, 0xab2d582b248c97df),
    ),
    (
        "Encoded",
        (6688, 0xe0867cdf7c49be83),
        (3190, 0xf372d514f4a9c841),
    ),
    (
        "MixedEncoded",
        (4288, 0xe2b733eb4f436655),
        (2390, 0x0013c5cf1782d57b),
    ),
    (
        "Groups",
        (868, 0xb63cd5db14f7659f),
        (910, 0x39b1773e689c9c61),
    ),
    (
        "Broken",
        (2288, 0xfa9675dc2ec69f9b),
        (1790, 0xb0fce88e51e5890f),
    ),
    (
        "LongAtoms",
        (11288, 0x0659fb2e3ddf7815),
        (11390, 0x8c23585ad281d561),
    ),
    (
        "Quoted",
        (3578, 0x74bcf0d2b52a8a7b),
        (3180, 0x91fc5421f666d823),
    ),
    (
        "Comment",
        (32784, 0x64ef3705b7c66460),
        (16, 0x862f0a6934f43626),
    ),
    (
        "UnicodeAddr",
        (1788, 0xa9d41d428a8dc65b),
        (1890, 0xfac4bac314fe723f),
    ),
    (
        "QuotedLocal",
        (2888, 0xbf06285c689084a5),
        (2990, 0xae2519e5560ba8c5),
    ),
    (
        "CFWSDomain",
        (2288, 0x1023dd973dded3af),
        (1790, 0x1fd1fed1443e6fcf),
    ),
    (
        "TrailingDot",
        (1788, 0x82573115ccfe543d),
        (1790, 0x1fd1fed1443e6fcf),
    ),
    (
        "Utf8Cjk",
        (3178, 0xea0a017a4d2e46e3),
        (2980, 0x0998cb1d9f91377f),
    ),
    (
        "UnknownCharset",
        (4388, 0x58b2e7dec9c4c263),
        (2390, 0x0013c5cf1782d57b),
    ),
    (
        "QuotedUtf8",
        (3788, 0x2026e8f22504d0cf),
        (3290, 0x39dde4cd5ca5c287),
    ),
    (
        "QuotedEncoded",
        (4588, 0xd02775e53ae154db),
        (2490, 0x23a17e5da378601f),
    ),
    (
        "Utf8Emoji",
        (3178, 0x48b5d3a10b43e77b),
        (2980, 0xdbac4eee675221bb),
    ),
    (
        "EncodedCjk",
        (4388, 0x11fef2d70a7ed869),
        (2690, 0xb88df8ade503eb2f),
    ),
    (
        "InvalidEncoded",
        (4488, 0x327ac90f849575d3),
        (2690, 0xe9255eeec46ececf),
    ),
    (
        "LongName",
        (104388, 0x4a7ce77bb9d796f5),
        (104190, 0x61d8fe1edc8b190f),
    ),
    (
        "LongUtf8",
        (104588, 0x805c0c7efdae44a5),
        (104390, 0xacf7318fcd99906f),
    ),
    (
        "LongEquals",
        (104588, 0xa0debdf3a2f14379),
        (104390, 0x1a78c80665c02bcb),
    ),
    (
        "CommentNames",
        (2678, 0x4e977ae6ad7f4b57),
        (2480, 0x9e0821d1bbd1d6ab),
    ),
    (
        "CommentTabs",
        (3578, 0x2e18432dd8e9714b),
        (3180, 0xdb8240dc3230f083),
    ),
    (
        "CommentUtf8",
        (3678, 0x3c1108152728238b),
        (3480, 0xab2d582b248c97df),
    ),
    (
        "CommentEncoded",
        (6688, 0x56f6128d086cdd39),
        (3190, 0xf372d514f4a9c841),
    ),
    (
        "LongComment",
        (104388, 0xa497aa1fe48d87f3),
        (104190, 0x61d8fe1edc8b190f),
    ),
    (
        "LongCommentUtf8",
        (104588, 0x9b421f8223ab0e63),
        (104390, 0xacf7318fcd99906f),
    ),
    (
        "CommentUtf8Tabs",
        (3878, 0x151ab23008c67743),
        (3480, 0xab2d582b248c97df),
    ),
    (
        "CommentEncodedTabs",
        (6688, 0x892d6633674f4c13),
        (3190, 0xf372d514f4a9c841),
    ),
    (
        "CommentUnknownCharsetTabs",
        (4588, 0x5a22b2cd9916ab45),
        (2390, 0x0013c5cf1782d57b),
    ),
    (
        "LongCommentUtf8Tabs",
        (104688, 0x9eadc7e31509715f),
        (104490, 0xc1f1e4c0d82ef3b5),
    ),
    (
        "LongCommentEqualsTabs",
        (104688, 0xf9296830c323b0e3),
        (104490, 0x27bfc94a55819de9),
    ),
    (
        "EscapedName",
        (3878, 0x39b81ab203381143),
        (3280, 0x6c23caade6c1b8af),
    ),
    (
        "FoldedName",
        (3678, 0xbed93cd0b6f18e7f),
        (3080, 0xcced5cbca972ceb3),
    ),
    (
        "LongQuotedName",
        (104588, 0x5b87565877b2d1b9),
        (104190, 0x61d8fe1edc8b190f),
    ),
    (
        "LongQuotedUtf8",
        (104788, 0x52a63bf05b06b615),
        (104390, 0xacf7318fcd99906f),
    ),
    (
        "LongQuotedEquals",
        (104788, 0x362b4b439786cf81),
        (104390, 0x1a78c80665c02bcb),
    ),
];

pub fn workload(name: &'static str) -> Workload {
    let text = match name {
        "Single" => "Joe Sixpack <joe@example.com>".into(),
        "Bare" => "first.last@example.com".into(),
        "Comment" => format!("({}) a@example.com", "x".repeat(32768)),
        "Groups" => (0..20)
            .map(|i| format!("Group {i}: a{i}@example.com, b{i}@example.com;"))
            .collect::<Vec<_>>()
            .join(", "),
        _ => (0..100)
            .map(|i| match name {
                "Recipients" => format!("User {i} <u{i}@example.com>"),
                "Utf8" => format!("Ren\u{e9}e M\u{fc}ller {i} <u{i}@example.com>"),
                "Utf8Cjk" => format!("\u{674e}\u{5c0f}\u{9f8d} {i} <u{i}@example.com>"),
                "Utf8Emoji" => format!("Name \u{1f600} {i} <u{i}@example.com>"),
                "EncodedCjk" => format!("=?utf-8?b?5p2O5bCP6b6N?= <u{i}@example.com>"),
                "InvalidEncoded" => format!("=?utf-8?q?Ren=C3=A9e=FF?= <u{i}@example.com>"),
                "Encoded" => {
                    format!("=?utf-8?q?Ren=C3=A9e?= =?utf-8?q?_M=C3=BCller?= <u{i}@example.com>")
                }
                "MixedEncoded" => format!(
                    "{} <u{i}@example.com>",
                    if i % 2 == 0 {
                        "=?utf-8?q?Ren=C3=A9e?="
                    } else {
                        "=?iso-8859-1?q?Ren=E9e?="
                    }
                ),
                "UnknownCharset" => format!("=?unknown?q?Ren=C3=A9e?= <u{i}@example.com>"),
                "Broken" => format!("<<<u{i}@example.com>>>"),
                "LongAtoms" => format!("{}{i}@{}.example.com", "a".repeat(48), "b".repeat(48)),
                "Quoted" => format!("\"Last, First {i}\" <u{i}@example.com>"),
                "QuotedUtf8" => format!("\"Ren\u{e9}e\\\" M\u{fc}ller\" <u{i}@example.com>"),
                "QuotedEncoded" => format!("\"=?utf-8?b?IlxOYW1lIg==?=\" <u{i}@example.com>"),
                "EscapedName" => format!("\"First\\\" Last\\\\ {i}\" <u{i}@example.com>"),
                "FoldedName" => format!("\"First\r\n Last {i}\" <u{i}@example.com>"),
                "LongQuotedName" => format!("\"{}\" <u{i}@example.com>", "x".repeat(1024)),
                "LongQuotedUtf8" => format!("\"{}\u{e9}\" <u{i}@example.com>", "x".repeat(1024)),
                "LongQuotedEquals" => format!("\"{}=?\" <u{i}@example.com>", "x".repeat(1024)),
                "LongName" => format!("{} <u{i}@example.com>", "x".repeat(1024)),
                "LongUtf8" => format!("{}\u{e9} <u{i}@example.com>", "x".repeat(1024)),
                "LongEquals" => format!("{}=? <u{i}@example.com>", "x".repeat(1024)),
                "CommentNames" => format!("u{i}@example.com (User {i})"),
                "CommentTabs" => format!("u{i}@example.com (\tFirst\t Last {i}\t)"),
                "CommentUtf8" => format!("u{i}@example.com (Ren\u{e9}e M\u{fc}ller {i})"),
                "CommentEncoded" => {
                    format!("u{i}@example.com (=?utf-8?q?Ren=C3=A9e?= =?utf-8?q?_M=C3=BCller?=)")
                }
                "CommentUtf8Tabs" => format!("u{i}@example.com (\tRen\u{e9}e\tM\u{fc}ller {i}\t)"),
                "CommentEncodedTabs" => {
                    format!("u{i}@example.com (=?utf-8?q?Ren=C3=A9e?=\t=?utf-8?q?_M=C3=BCller?=)")
                }
                "CommentUnknownCharsetTabs" => {
                    format!("u{i}@example.com (\t=?unknown?q?Ren=C3=A9e?=\t)")
                }
                "LongCommentUtf8Tabs" => format!("u{i}@example.com ({}\t\u{e9})", "x".repeat(1024)),
                "LongCommentEqualsTabs" => format!("u{i}@example.com ({}\t=?)", "x".repeat(1024)),
                "LongComment" => format!("u{i}@example.com ({})", "x".repeat(1024)),
                "LongCommentUtf8" => format!("u{i}@example.com ({}\u{e9})", "x".repeat(1024)),
                "UnicodeAddr" => format!("\u{e9}{i}@\u{4e2d}.example"),
                "QuotedLocal" => format!("\"first last {i}\"@example.com"),
                "CFWSDomain" => format!("a{i}@example (c) . com"),
                "TrailingDot" => format!("a{i}@example.com."),
                _ => panic!("unknown address workload: {name}"),
            })
            .collect::<Vec<_>>()
            .join(", "),
    };
    Workload { name, text }
}

pub fn parse_bytes(case: &Workload) -> AddressList {
    AddressList::parse(black_box(case.text.as_bytes())).unwrap()
}

pub fn parse_string(case: &Workload) -> AddressList {
    black_box(case.text.as_str()).parse().unwrap()
}

pub fn decode_text() -> rfc2047::DecodedHeader {
    rfc2047::decode_text(black_box(RFC2047_INPUT))
}

pub fn decode_phrase() -> rfc2047::DecodedHeader {
    rfc2047::decode_phrase(black_box(RFC2047_INPUT))
}

pub fn check_rfc2047() {
    let expected = "I'm so happy! \u{1f600} I love MIME so much \u{2764}\u{fe0f}\u{200d}\u{1f525}! Isn't it great?";
    for decoded in [decode_text(), decode_phrase()] {
        assert_eq!(decoded.text, expected);
        assert!(decoded.issues.is_empty());
    }
}

pub fn address_fingerprint(addresses: &AddressList) -> (usize, u64) {
    fn append(addresses: &AddressList, output: &mut Vec<u8>) {
        for address in addresses.as_slice() {
            match address {
                Address::Mailbox(mailbox) => {
                    output.push(b'M');
                    output.extend_from_slice(mailbox.display_name().as_bytes());
                    output.push(0);
                    output.extend_from_slice(mailbox.address().as_str().as_bytes());
                    output.push(0);
                }
                Address::Group(group) => {
                    output.push(b'G');
                    output.extend_from_slice(group.display_name().as_bytes());
                    output.push(0);
                    append(group.members(), output);
                    output.push(b'E');
                }
            }
        }
    }
    let mut output = Vec::new();
    append(addresses, &mut output);
    fingerprint(&output)
}

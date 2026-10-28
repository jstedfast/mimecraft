// Adapted from MimeKit's UnitTests/Text/HtmlTokenizerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, Read};

use mimecraft::text::{
    HtmlAttribute, HtmlNamespace, HtmlSourcePosition, HtmlToken, HtmlTokenizer, HtmlTokenizerError,
    HtmlTokenizerOptions, HtmlTokenizerReader, HtmlTokenizerResult,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct DocTypeCases {
    cases: Vec<DocTypeCase>,
}

#[derive(Deserialize)]
struct DocTypeCase {
    name: String,
    content: String,
    assertions: std::collections::BTreeMap<String, serde_json::Value>,
}

#[test]
fn canonical_doctype_metadata() {
    let cases: DocTypeCases =
        serde_json::from_str(include_str!("data/html/tokenizer-doctypes.json")).unwrap();
    for case in cases.cases {
        let tokens = tokenize(&case.content, Default::default());
        let HtmlToken::DocType(doc) = &tokens[0] else {
            panic!("{}", case.name)
        };
        for (property, expected) in case.assertions {
            let actual = match property.as_str() {
                "ForceQuirksMode" => serde_json::json!(doc.force_quirks_mode),
                "Name" => serde_json::json!(doc.name()),
                "PublicKeyword" => serde_json::json!(doc.public_keyword()),
                "SystemKeyword" => serde_json::json!(doc.system_keyword()),
                "PublicIdentifier" => serde_json::json!(doc.public_identifier()),
                "SystemIdentifier" => serde_json::json!(doc.system_identifier()),
                _ => panic!("unhandled DOCTYPE property: {property}"),
            };
            assert_eq!(actual, expected, "{}: {property}", case.name);
        }
    }
}

#[test]
fn bad_utf8_prefixes_fail_before_eof() {
    for input in [
        &b"\xe2x"[..],
        &b"\xed\xa0"[..],
        &b"\xf0\x80"[..],
        &b"\xf4\x90"[..],
    ] {
        let mut tokenizer = HtmlTokenizer::new();
        tokenizer.push(input).unwrap();
        assert!(matches!(
            tokenizer.next_token(),
            Err(HtmlTokenizerError::InvalidUtf8(_))
        ));
    }
}

#[test]
fn reader_interrupted_and_into_parts_resume() {
    struct InterruptOnce<'a> {
        bytes: &'a [u8],
        interrupted: bool,
    }
    impl Read for InterruptOnce<'_> {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            unreachable!("adapter uses fill_buf")
        }
    }
    impl BufRead for InterruptOnce<'_> {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if !self.interrupted {
                self.interrupted = true;
                Err(io::ErrorKind::Interrupted.into())
            } else {
                Ok(self.bytes)
            }
        }
        fn consume(&mut self, count: usize) {
            self.bytes = &self.bytes[count..];
        }
    }
    let input = "<p a='&amp;'>\u{1f600}</p>";
    let expected = tokenize(input, Default::default());
    let mut reader = HtmlTokenizerReader::new(InterruptOnce {
        bytes: input.as_bytes(),
        interrupted: false,
    });
    let mut actual = vec![reader.read_next_token().unwrap().unwrap()];
    // The source buffer was copied, but extraction must retain unconsumed bytes
    // in the engine so callers can transfer it to a different I/O driver.
    let (source, mut engine) = reader.into_parts();
    assert!(source.bytes.is_empty());
    engine.finish();
    assert_eq!(
        drain(&mut engine, &mut actual),
        HtmlTokenizerResult::EndOfInput
    );
    assert_eq!(actual, expected);
}

fn drain(tokenizer: &mut HtmlTokenizer, tokens: &mut Vec<HtmlToken>) -> HtmlTokenizerResult {
    loop {
        match tokenizer.next_token().unwrap() {
            HtmlTokenizerResult::Token(token) => tokens.push(token),
            result => return result,
        }
    }
}

fn tokenize(input: &str, options: HtmlTokenizerOptions) -> Vec<HtmlToken> {
    let mut tokenizer = HtmlTokenizer::with_options(options).unwrap();
    tokenizer.push(input.as_bytes()).unwrap();
    tokenizer.finish();
    let mut tokens = Vec::new();
    assert_eq!(
        drain(&mut tokenizer, &mut tokens),
        HtmlTokenizerResult::EndOfInput
    );
    tokens
}

fn positioned_tokens(
    input: &str,
    chunk_size: usize,
    options: HtmlTokenizerOptions,
) -> Vec<(HtmlToken, HtmlSourcePosition)> {
    let mut tokenizer = HtmlTokenizer::with_options(options).unwrap();
    let mut tokens = Vec::new();
    for chunk in input.as_bytes().chunks(chunk_size).map(Some).chain([None]) {
        if let Some(chunk) = chunk {
            tokenizer.push(chunk).unwrap();
        } else {
            tokenizer.finish();
        }
        loop {
            match tokenizer.next_token().unwrap() {
                HtmlTokenizerResult::Token(token) => tokens.push((token, tokenizer.position())),
                HtmlTokenizerResult::NeedInput => {
                    assert!(chunk.is_some());
                    break;
                }
                HtmlTokenizerResult::EndOfInput => {
                    assert!(chunk.is_none());
                    assert_eq!(
                        tokenizer.position(),
                        HtmlSourcePosition {
                            offset: input.len() as u64,
                            line: input.bytes().filter(|&b| b == b'\n').count() as u64 + 1,
                            column: input.rsplit('\n').next().unwrap().chars().count() as u64 + 1,
                        }
                    );
                    return tokens;
                }
            }
        }
    }
    unreachable!("explicit EOF")
}

#[test]
fn long_runs_preserve_tokens_and_scalar_positions_across_chunks() {
    let text = "ordinary \u{e9}\u{1f600}\r\n".repeat(300);
    let name = format!("data-{}\u{e9}", "x".repeat(4100));
    let input = format!(
        "<!DOCTYPE html PUBLIC '{text}' \"{text}\">\
         <p {name}='{text}&amp;\0' CLASS=ordinary class=''>\
         {text}&amp;\0</p><!--{text}--><?{text}>\
         <textarea>{text}&amp;\0</textarea>\
         <style>{text}\0</style>\
         <script><!--{text}<script>{text}</script>{text}--></script>\
         <svg><![CDATA[{text}\0]]]></svg><plaintext>{text}\0<&"
    );
    for decode in [false, true] {
        let options = HtmlTokenizerOptions {
            decode_character_references: decode,
            ..Default::default()
        };
        let expected = positioned_tokens(&input, 1, options);
        for chunk_size in [2, 3, 7, 17, 127, 4096, 8191, input.len()] {
            assert_eq!(
                positioned_tokens(&input, chunk_size, options),
                expected,
                "decode={decode}, chunk_size={chunk_size}"
            );
        }
    }
}

#[test]
fn complete_tag_fallback_preserves_prior_attributes_and_positions() {
    for suffix in [
        "b='two>three'>",
        "b='two' / c=three>",
        "b='two' /=three>",
        "=three>",
        "b='two'c=three>",
        "b=>",
        "b=three/>",
        "b=''>",
        "b='&amp;'>",
        "b='\0'>",
        "b='\u{1f600}'>",
        "b='unfinished",
        "b='two'/",
    ] {
        let input = format!("<p id='one' a=first {suffix}");
        for ignore in [false, true] {
            let options = HtmlTokenizerOptions {
                ignore_truncated_tags: ignore,
                ..Default::default()
            };
            let expected = positioned_tokens(&input, 1, options);
            for chunk_size in [2, 7, input.len()] {
                assert_eq!(
                    positioned_tokens(&input, chunk_size, options),
                    expected,
                    "{input}"
                );
            }
        }
    }
}

#[test]
fn validated_spans_defer_utf8_errors_until_their_source_position() {
    let ordinary = "x\u{e9}\u{1f600}\r\n".repeat(1024);
    for prefix in [
        format!("<p>{ordinary}</p>"),
        format!("<p>{ordinary}</p><a value='{ordinary}"),
        format!("<p>{ordinary}</p><!--{ordinary}"),
    ] {
        for invalid in [&b"\xff"[..], &b"\xed\xa0"[..], &b"\xf0\x9f"[..]] {
            let input = [prefix.as_bytes(), invalid].concat();
            let mut expected = None;
            for chunk_size in [1, 7, 4096, input.len()] {
                let mut tokenizer = HtmlTokenizer::new();
                let mut tokens = Vec::new();
                let mut failure = None;
                'feed: for chunk in input.chunks(chunk_size).map(Some).chain([None]) {
                    if let Some(chunk) = chunk {
                        tokenizer.push(chunk).unwrap();
                    } else {
                        tokenizer.finish();
                    }
                    loop {
                        match tokenizer.next_token() {
                            Ok(HtmlTokenizerResult::Token(token)) => {
                                tokens.push((token, tokenizer.position()));
                            }
                            Ok(HtmlTokenizerResult::NeedInput) => break,
                            Err(HtmlTokenizerError::InvalidUtf8(error)) => {
                                let position = tokenizer.position();
                                assert_eq!(position.offset, prefix.len() as u64);
                                let sticky = HtmlTokenizerError::InvalidUtf8(error);
                                assert_eq!(tokenizer.next_token(), Err(sticky));
                                assert_eq!(tokenizer.push(b"more"), Err(sticky));
                                tokenizer.finish();
                                assert_eq!(tokenizer.next_token(), Err(sticky));
                                failure = Some((error, position));
                                break 'feed;
                            }
                            result => panic!("expected invalid UTF-8, got {result:?}"),
                        }
                    }
                }
                let actual = (tokens, failure.expect("UTF-8 error"));
                if let Some(expected) = &expected {
                    assert_eq!(&actual, expected, "chunk_size={chunk_size}");
                } else {
                    expected = Some(actual);
                }
            }
        }
    }
}

// MimeKit's DescribeTokens merges adjacent data of all three data kinds.
// Exact token variants and boundaries are checked separately across partitions.
fn describe(tokens: &[HtmlToken]) -> String {
    let mut output = String::new();
    let mut previous_data = false;
    for token in tokens {
        let data = match token {
            HtmlToken::Data(data) => Some(data.data()),
            HtmlToken::CData(data) | HtmlToken::ScriptData(data) => Some(data.as_str()),
            _ => None,
        };
        if let Some(data) = data {
            if previous_data {
                output.pop();
            } else {
                if !output.is_empty() {
                    output.push(' ');
                }
                output.push_str("Data(");
            }
            output.push_str(data);
            output.push(')');
            previous_data = true;
            continue;
        }
        previous_data = false;
        if !output.is_empty() {
            output.push(' ');
        }
        match token {
            HtmlToken::Tag(tag) => {
                output.push_str(if tag.is_end_tag() { "EndTag(" } else { "Tag(" });
                output.push_str(tag.name());
                for attribute in tag.attributes() {
                    output.push(' ');
                    output.push_str(attribute.name());
                    if let Some(value) = attribute.value() {
                        output.push_str("=\"");
                        output.push_str(value);
                        output.push('"');
                    }
                }
                output.push_str(if tag.is_empty_element() { "/)" } else { ")" });
            }
            HtmlToken::Comment(comment) => {
                output.push_str(if comment.is_bogus_comment() {
                    "Bogus("
                } else {
                    "Comment("
                });
                output.push_str(comment.comment());
                output.push(')');
            }
            HtmlToken::DocType(_) => output.push_str("DocType"),
            _ => unreachable!(),
        }
    }
    output
}

#[derive(Deserialize)]
struct Cases {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    content: String,
    expected: String,
    decode: bool,
    scripting: bool,
    depth: usize,
    suffix: bool,
}

fn cases() -> Vec<Case> {
    serde_json::from_str::<Cases>(include_str!("data/html/tokenizer-cases.json"))
        .unwrap()
        .cases
}

#[test]
fn canonical_context_recovery_and_adoption_cases() {
    for case in cases() {
        let options = HtmlTokenizerOptions {
            decode_character_references: case.decode,
            scripting_enabled: case.scripting,
            max_element_depth: case.depth,
            ..Default::default()
        };
        let tokens = tokenize(&case.content, options);
        let actual = describe(&tokens);
        if case.suffix {
            assert!(actual.ends_with(&case.expected), "{}: {actual}", case.name);
        } else {
            assert_eq!(actual, case.expected, "{}", case.name);
        }

        let mut reader =
            HtmlTokenizerReader::with_options(case.content.as_bytes(), options).unwrap();
        let mut read = Vec::new();
        while let Some(token) = reader.read_next_token().unwrap() {
            read.push(token);
        }
        assert_eq!(read, tokens, "reader: {}", case.name);
        // Every byte boundary, including boundaries inside references and markup.
        for split in 0..=case.content.len() {
            let mut tokenizer = HtmlTokenizer::with_options(options).unwrap();
            let mut fragmented = Vec::new();
            tokenizer.push(&case.content.as_bytes()[..split]).unwrap();
            assert_eq!(
                drain(&mut tokenizer, &mut fragmented),
                HtmlTokenizerResult::NeedInput
            );
            assert_eq!(
                drain(&mut tokenizer, &mut fragmented),
                HtmlTokenizerResult::NeedInput
            );
            tokenizer.push(&case.content.as_bytes()[split..]).unwrap();
            tokenizer.finish();
            assert_eq!(
                drain(&mut tokenizer, &mut fragmented),
                HtmlTokenizerResult::EndOfInput
            );
            assert_eq!(fragmented, tokens, "{}, split={split}", case.name);
        }
    }
}

#[test]
fn explicit_eof_and_empty_chunks_are_distinct() {
    let mut tokenizer = HtmlTokenizer::new();
    assert_eq!(
        tokenizer.next_token().unwrap(),
        HtmlTokenizerResult::NeedInput
    );
    tokenizer.push(b"").unwrap();
    tokenizer.push(b"<img src='").unwrap();
    assert_eq!(
        tokenizer.next_token().unwrap(),
        HtmlTokenizerResult::NeedInput
    );
    assert!(!tokenizer.is_eof());
    tokenizer.finish();
    let HtmlTokenizerResult::Token(HtmlToken::Data(data)) = tokenizer.next_token().unwrap() else {
        panic!()
    };
    assert_eq!(data.data(), "<img src='");
    assert_eq!(
        tokenizer.next_token().unwrap(),
        HtmlTokenizerResult::EndOfInput
    );
    assert!(tokenizer.is_eof());
    tokenizer.finish();
    assert_eq!(tokenizer.push(b"x"), Err(HtmlTokenizerError::InputFinished));
    assert_eq!(
        tokenizer.next_token().unwrap(),
        HtmlTokenizerResult::EndOfInput
    );
}

#[test]
fn bytewise_utf8_and_positions() {
    let input = "\u{feff}<p a='\u{1f600}'>caf\u{e9}\r\n&amp;</p>";
    let expected = tokenize(input, HtmlTokenizerOptions::default());
    let mut tokenizer = HtmlTokenizer::new();
    let mut actual = Vec::new();
    for byte in input.bytes() {
        tokenizer.push(&[byte]).unwrap();
        assert_eq!(
            drain(&mut tokenizer, &mut actual),
            HtmlTokenizerResult::NeedInput
        );
    }
    tokenizer.finish();
    assert_eq!(
        drain(&mut tokenizer, &mut actual),
        HtmlTokenizerResult::EndOfInput
    );
    assert_eq!(actual, expected);
    assert_eq!(
        tokenizer.position(),
        HtmlSourcePosition {
            offset: input.len() as u64,
            line: 2,
            column: 10,
        }
    );
    assert!(matches!(&actual[0], HtmlToken::Data(data) if data.data() == "\u{feff}"));
}

#[test]
fn invalid_and_truncated_utf8_fail_without_replacement() {
    for input in [
        &b"\xff"[..],
        &b"\xc0\xaf"[..],
        &b"\xed\xa0\x80"[..],
        &b"\xf4\x90\x80\x80"[..],
        &b"\xe2\x82"[..],
    ] {
        let mut tokenizer = HtmlTokenizer::new();
        tokenizer.push(input).unwrap();
        tokenizer.finish();
        let error = tokenizer.next_token().unwrap_err();
        assert!(matches!(error, HtmlTokenizerError::InvalidUtf8(_)));
        assert_eq!(tokenizer.next_token(), Err(error));
        assert_eq!(tokenizer.push(b"more"), Err(error));
        let mut reader = HtmlTokenizerReader::new(input);
        assert_eq!(
            reader.read_next_token().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    let mut tokenizer = HtmlTokenizer::new();
    tokenizer.push(b"\xe2\x82").unwrap();
    assert_eq!(
        tokenizer.next_token().unwrap(),
        HtmlTokenizerResult::NeedInput
    );
    tokenizer.push(b"\xac").unwrap();
    tokenizer.finish();
    let HtmlTokenizerResult::Token(HtmlToken::Data(data)) = tokenizer.next_token().unwrap() else {
        panic!()
    };
    assert_eq!(data.data(), "\u{20ac}");
}

#[test]
fn named_numeric_and_attribute_references() {
    let input = "&#65&#x42;&#x110000;&#0;&#xD800;&#x80;&#0000000000000000000000000000000000000067;&#99999999999999999999;&#;&#x;";
    assert_eq!(
        describe(&tokenize(input, Default::default())),
        "Data(AB\u{fffd}\u{fffd}\u{fffd}\u{20ac}C\u{fffd}&#;&#x;)"
    );
    // A legacy name without ';' is not decoded in an attribute when followed
    // by '=' or ASCII alphanumerics.
    let input = "<a href=\"?a=1&not=2&noti;&notin;&not;&not!\">&noti;&notin</a>";
    assert_eq!(
        describe(&tokenize(input, Default::default())),
        "Tag(a href=\"?a=1&not=2&noti;\u{2209}\u{ac}\u{ac}!\") Data(\u{ac}i;\u{ac}in) EndTag(a)"
    );
    for decode in [false, true] {
        let options = HtmlTokenizerOptions {
            decode_character_references: decode,
            ..Default::default()
        };
        let tokens = tokenize("<p a=&amp;>&amp;<title>&lt;\0</title>", options);
        assert_eq!(
            describe(&tokens),
            if decode {
                "Tag(p a=\"&\") Data(&) Tag(title) Data(<\u{fffd}) EndTag(title)"
            } else {
                "Tag(p a=\"&\") Data(&amp;) Tag(title) Data(&lt;\u{fffd}) EndTag(title)"
            }
        );
    }
}

#[test]
fn truncated_tags_in_every_attribute_state() {
    for input in [
        "<a",
        "<a ",
        "<a b",
        "<a b ",
        "<a/",
        "<a /",
        "<a b/",
        "<a b /",
        "<a b=",
        "<a b= ",
        "<a b='x",
        "<a b=\"x",
        "<a b=x",
        "<a b=&",
        "<a b=&amp",
        "<a b='&",
        "<a b='&amp",
        "<a b=\"v\"",
        "<a b=\"v\"/",
        "<a b=x ",
        "<style>x</style ",
        "<script>x</script/",
    ] {
        for ignore in [false, true] {
            let options = HtmlTokenizerOptions {
                ignore_truncated_tags: ignore,
                ..Default::default()
            };
            let tokens = tokenize(input, options);
            if input.starts_with("<a") {
                assert_eq!(tokens.len(), usize::from(!ignore), "{input}");
                if let Some(HtmlToken::Data(data)) = tokens.first() {
                    assert_eq!(data.data(), input);
                }
            } else {
                assert_eq!(tokens.len(), if ignore { 2 } else { 3 }, "{input}");
            }
            for split in 0..=input.len() {
                let mut tokenizer = HtmlTokenizer::with_options(options).unwrap();
                let mut fragmented = Vec::new();
                tokenizer.push(&input.as_bytes()[..split]).unwrap();
                assert_eq!(
                    drain(&mut tokenizer, &mut fragmented),
                    HtmlTokenizerResult::NeedInput
                );
                tokenizer.push(&input.as_bytes()[split..]).unwrap();
                tokenizer.finish();
                drain(&mut tokenizer, &mut fragmented);
                assert_eq!(fragmented, tokens, "{input}, split={split}");
            }
        }
    }
    // A bare '<' or '</' is data, not an ignored truncated tag.
    for input in ["<", "</"] {
        assert_eq!(
            describe(&tokenize(
                input,
                HtmlTokenizerOptions {
                    ignore_truncated_tags: true,
                    ..Default::default()
                }
            )),
            format!("Data({input})")
        );
    }
}

#[test]
fn comments_nulls_and_eof_recovery() {
    for (input, expected) in [
        ("<!", "Bogus()"),
        ("<!-", "Bogus(-)"),
        ("<!DOC>", "Bogus(DOC)"),
        ("<?a\0>", "Bogus(?a\u{fffd})"),
        ("</\0>", "Bogus(\u{fffd})"),
        ("<!--", "Comment()"),
        ("<!---", "Comment()"),
        ("<!----", "Comment()"),
        ("<!-->", "Comment()"),
        ("<!--->", "Comment()"),
        ("<!---->", "Comment()"),
        ("<!--x-", "Comment(x)"),
        ("<!--x--", "Comment(x)"),
        ("<!--x--!", "Comment(x)"),
        ("<!--x--!>", "Comment(x)"),
        ("<!--x--!y-->", "Comment(x--!y)"),
        ("<!--x--!->", "Comment(x--!->)"),
        ("<!--x\0y-->", "Comment(x\u{fffd}y)"),
        ("<!--x<!--y-->", "Comment(x<!--y)"),
        ("</>x", "Data(x)"),
    ] {
        assert_eq!(
            describe(&tokenize(input, Default::default())),
            expected,
            "{input}"
        );
    }
}

#[test]
fn doctype_recovery_and_keyword_isolation() {
    for (input, name, public, system, quirks) in [
        ("<!DOCTYPE", None, None, None, true),
        ("<!DOCTYPE >", None, None, None, true),
        ("<!doctype html>", Some("html"), None, None, false),
        ("<!DOCTYPE html", Some("html"), None, None, true),
        ("<!DOCTYPE h\0tml>", Some("h\u{fffd}tml"), None, None, false),
        ("<!DOCTYPE html PU>", Some("html"), None, None, true),
        ("<!DOCTYPE html SYSTEM>", Some("html"), None, None, true),
        (
            "<!DOCTYPE html PUBLIC 'a'>",
            Some("html"),
            Some("a"),
            None,
            false,
        ),
        (
            "<!DOCTYPE html PUBLIC 'a' 'b'>",
            Some("html"),
            Some("a"),
            Some("b"),
            false,
        ),
        (
            "<!DOCTYPE html PUBLIC'a''b'>",
            Some("html"),
            Some("a"),
            Some("b"),
            false,
        ),
        (
            "<!DOCTYPE html SYSTEM 'x'>",
            Some("html"),
            None,
            Some("x"),
            false,
        ),
        (
            "<!DOCTYPE html SYSTEM 'x'",
            Some("html"),
            None,
            Some("x"),
            true,
        ),
        (
            "<!DOCTYPE html SYSTEM 'x' junk",
            Some("html"),
            None,
            Some("x"),
            false,
        ),
        (
            "<!DOCTYPE html PUBLIC 'x>",
            Some("html"),
            Some("x"),
            None,
            true,
        ),
        (
            "<!DOCTYPE html SYSTEM 'x>",
            Some("html"),
            None,
            Some("x"),
            true,
        ),
    ] {
        let tokens = tokenize(input, Default::default());
        let [HtmlToken::DocType(doc)] = tokens.as_slice() else {
            panic!("{input}: {tokens:?}")
        };
        assert_eq!(doc.name(), name, "{input}");
        assert_eq!(doc.public_identifier(), public, "{input}");
        assert_eq!(doc.system_identifier(), system, "{input}");
        assert_eq!(doc.force_quirks_mode, quirks, "{input}");
    }
    for input in [
        "<!DOCTYPE l PUBLIC\"><k g=&>",
        "<!DOCTYPE l PUBLIC '><k g=&>",
        "<!DOCTYPE l SYSTEM \"><k g=&>",
        "<!DOCTYPE l PUBLIC \"a\" '><k g=&>",
        "<!DOCTYPE l PU><k g=&>",
    ] {
        let tokens = tokenize(input, Default::default());
        assert_eq!(tokens.len(), 2, "{input}");
        let HtmlToken::Tag(tag) = &tokens[1] else {
            panic!()
        };
        assert_eq!(tag.name(), "k");
        assert_eq!(tag.attributes()[0].name(), "g");
        assert_eq!(tag.attributes()[0].value(), Some("&"));
    }
    let tokens = tokenize("<!DoCtYpE HTML PuBlIc 'a' \"b\">", Default::default());
    assert_eq!(tokens[0].to_html(), "<!DoCtYpE HTML PuBlIc 'a' \"b\">");
}

#[test]
fn malformed_attribute_names_preserve_reparse_boundaries() {
    let tokens = tokenize("<a b/=c d='<&' D=x>", Default::default());
    let HtmlToken::Tag(tag) = &tokens[0] else {
        panic!()
    };
    assert_eq!(tag.attributes().len(), 4);
    assert_eq!(tag.attributes()[1].name(), "=c");
    assert_eq!(tokens[0].to_html(), "<a b=\"\" =c d=\"&lt;&amp;\" D=\"x\">");
    assert!(HtmlAttribute::new("=c", None).is_err());
    let reparsed = tokenize(&tokens[0].to_html(), Default::default());
    let HtmlToken::Tag(tag) = &reparsed[0] else {
        panic!()
    };
    assert_eq!(tag.attributes()[0].value(), Some(""));
    assert_eq!(tag.attributes()[1].name(), "=c");
}

#[test]
fn depth_overflow_literal_data_is_encoded_even_without_decoding() {
    let options = HtmlTokenizerOptions {
        max_element_depth: 3,
        decode_character_references: false,
        ..Default::default()
    };
    let tokens = tokenize("<div><span><b><i><img src=x>&amp;", options);
    assert_eq!(
        tokens.last().unwrap().to_html(),
        "&lt;img src=x&gt;&amp;amp;"
    );
    let tokens = tokenize("<plaintext>&amp;\0<b>", Default::default());
    assert_eq!(tokens.last().unwrap().to_html(), "&amp;\u{fffd}<b>");
}

#[test]
fn data_serialization_protects_trailing_less_than() {
    let tokens = tokenize(
        "<</>c>",
        HtmlTokenizerOptions {
            decode_character_references: false,
            ..Default::default()
        },
    );
    assert_eq!(
        tokens.iter().map(HtmlToken::to_html).collect::<String>(),
        "&lt;c>"
    );
}

#[test]
fn namespaces_and_structural_observations() {
    let mut reader =
        HtmlTokenizerReader::from_text("<html xmlns='http://www.w3.org/2000/svg'><svg><g><div>");
    reader.tokenizer_mut().set_track_closed_elements(true);
    assert!(reader.read_next_token().unwrap().is_some());
    assert_eq!(reader.tokenizer().html_namespace(), HtmlNamespace::Svg);
    while reader.read_next_token().unwrap().is_some() {}
    let tree = reader.tokenizer().tree_construction();
    assert!(tree.last_tag().unwrap().broke_out_of_foreign_content);
    assert_eq!(
        tree.closed_elements()
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["g", "svg"]
    );
}

struct PausingReader<'a> {
    input: &'a [u8],
    offset: usize,
    pause: usize,
    failed: bool,
}

impl Read for PausingReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let bytes = self.fill_buf()?;
        let count = bytes.len().min(output.len());
        output[..count].copy_from_slice(&bytes[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for PausingReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.offset == self.pause && !self.failed {
            self.failed = true;
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        Ok(&self.input[self.offset..(self.offset + 1).min(self.input.len())])
    }
    fn consume(&mut self, count: usize) {
        self.offset += count;
    }
}

#[test]
fn reader_errors_preserve_partial_tokens_and_utf8() {
    let input = "<!DoCtYpE html PUBLIC 'x'><p a='&notin;\u{1f600}'>\u{e9}&amp;</p><script><!--<script></script>--></script><svg><![CDATA[x]]></svg>";
    let expected = tokenize(input, Default::default());
    for pause in 0..=input.len() {
        let source = PausingReader {
            input: input.as_bytes(),
            offset: 0,
            pause,
            failed: false,
        };
        let mut reader = HtmlTokenizerReader::new(source);
        let mut tokens = Vec::new();
        loop {
            match reader.read_next_token() {
                Ok(Some(token)) => tokens.push(token),
                Ok(None) => break,
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::WouldBlock),
            }
        }
        assert_eq!(tokens, expected, "pause={pause}");
        assert!(reader.get_ref().failed);
        let (_, mut engine) = reader.into_parts();
        assert_eq!(
            engine.next_token().unwrap(),
            HtmlTokenizerResult::EndOfInput
        );
    }
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn fixture_description(tokens: &[HtmlToken]) -> String {
    let mut description = String::new();
    for token in tokens {
        description.push_str(&format!("{:?}: ", token.kind()));
        match token {
            HtmlToken::Data(data) => description.push_str(&escaped_data(data.data())),
            HtmlToken::CData(data) | HtmlToken::ScriptData(data) => {
                description.push_str(&escaped_data(data))
            }
            HtmlToken::Comment(comment) => {
                description.push_str(&comment.comment().replace("\r\n", "\n"))
            }
            HtmlToken::Tag(tag) => {
                description.push('<');
                if tag.is_end_tag() {
                    description.push('/');
                }
                description.push_str(tag.name());
                for attribute in tag.attributes() {
                    description.push(' ');
                    description.push_str(attribute.name());
                    if let Some(value) = attribute.value() {
                        description.push('=');
                        description.push_str(&quoted(value));
                    }
                }
                if tag.is_empty_element() {
                    description.push('/');
                }
                description.push('>');
            }
            HtmlToken::DocType(doc) => {
                if doc.force_quirks_mode {
                    description.push_str("<!-- force quirks mode -->");
                }
                description.push_str("<!DOCTYPE");
                if let Some(name) = doc.name() {
                    description.push(' ');
                    description.push_str(&name.to_uppercase());
                }
                if let Some(public) = doc.public_identifier() {
                    description.push_str(" PUBLIC ");
                    description.push_str(&quoted(public));
                    if let Some(system) = doc.system_identifier() {
                        description.push(' ');
                        description.push_str(&quoted(system));
                    }
                } else if let Some(system) = doc.system_identifier() {
                    description.push_str(" SYSTEM ");
                    description.push_str(&quoted(system));
                }
                description.push('>');
            }
        }
        description.push('\n');
    }
    description
}

fn escaped_data(data: &str) -> String {
    data.replace('\r', "")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\x0c', "\\f")
}

#[derive(Deserialize)]
struct Documents {
    documents: Vec<Document>,
}
#[derive(Deserialize)]
struct Document {
    name: String,
    content: String,
    tokens: String,
    output: String,
}

#[test]
fn canonical_document_tokens_and_serialization() {
    let documents: Documents =
        serde_json::from_str(include_str!("data/html/tokenizer-documents.json")).unwrap();
    for document in documents.documents {
        let expected = tokenize(&document.content, Default::default());
        let description = fixture_description(&expected);
        for (line, (actual, expected)) in
            description.lines().zip(document.tokens.lines()).enumerate()
        {
            assert_eq!(actual, expected, "{}, line={line}", document.name);
        }
        assert_eq!(
            description.lines().count(),
            document.tokens.lines().count(),
            "{}",
            document.name
        );
        let output = expected.iter().map(HtmlToken::to_html).collect::<String>();
        for (offset, (actual, expected)) in output.bytes().zip(document.output.bytes()).enumerate()
        {
            assert_eq!(
                actual, expected,
                "{}, serialized offset={offset}",
                document.name
            );
        }
        assert_eq!(output.len(), document.output.len(), "{}", document.name);
        for chunk_size in [1, 17, 4096] {
            let mut tokenizer = HtmlTokenizer::new();
            let mut actual = Vec::new();
            for chunk in document.content.as_bytes().chunks(chunk_size) {
                tokenizer.push(chunk).unwrap();
                assert_eq!(
                    drain(&mut tokenizer, &mut actual),
                    HtmlTokenizerResult::NeedInput
                );
            }
            tokenizer.finish();
            assert_eq!(
                drain(&mut tokenizer, &mut actual),
                HtmlTokenizerResult::EndOfInput
            );
            assert_eq!(
                actual, expected,
                "{}, chunk_size={chunk_size}",
                document.name
            );
        }
    }
}

#[derive(Deserialize)]
struct RecoveryInputs {
    inputs: Vec<RecoveryInput>,
}
#[derive(Deserialize)]
struct RecoveryInput {
    name: String,
    content: String,
}

#[test]
fn all_recovery_prefixes_suspend_and_resume() {
    let inputs: RecoveryInputs =
        serde_json::from_str(include_str!("data/html/tokenizer-recovery-inputs.json")).unwrap();
    for input in inputs.inputs {
        for end in (0..=input.content.len()).filter(|&end| input.content.is_char_boundary(end)) {
            let prefix = &input.content[..end];
            for ignore in [false, true] {
                let options = HtmlTokenizerOptions {
                    ignore_truncated_tags: ignore,
                    ..Default::default()
                };
                let expected = tokenize(prefix, options);
                let mut tokenizer = HtmlTokenizer::with_options(options).unwrap();
                let mut actual = Vec::new();
                for byte in prefix.bytes() {
                    tokenizer.push(&[byte]).unwrap();
                    assert_eq!(
                        drain(&mut tokenizer, &mut actual),
                        HtmlTokenizerResult::NeedInput
                    );
                    tokenizer.push(b"").unwrap();
                    assert_eq!(
                        drain(&mut tokenizer, &mut actual),
                        HtmlTokenizerResult::NeedInput
                    );
                }
                tokenizer.finish();
                assert_eq!(
                    drain(&mut tokenizer, &mut actual),
                    HtmlTokenizerResult::EndOfInput
                );
                assert_eq!(actual, expected, "{}, end={end}", input.name);
            }
        }
    }
}

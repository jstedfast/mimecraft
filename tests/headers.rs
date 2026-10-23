// Includes cases adapted from MimeKit's HeaderListTests.cs and MimeParserTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::num::NonZeroUsize;

use mimecraft::{Header, HeaderParser};

fn parse_chunks(input: &[u8], size: usize) -> (Vec<Header>, usize, Option<Vec<u8>>) {
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    let mut consumed = 0;
    for chunk in input.chunks(size) {
        let count = parser.parse(chunk, &mut headers).unwrap();
        consumed += count;
        assert_eq!(parser.parse(b"", &mut headers).unwrap(), 0);
        if parser.is_finished() {
            break;
        }
        assert_eq!(count, chunk.len());
    }
    parser.finish(&mut headers).unwrap();
    let count = headers.len();
    parser.finish(&mut headers).unwrap();
    assert_eq!(headers.len(), count);
    assert!(parser.is_finished());
    assert_eq!(
        parser.parse(b"Not-A-Header: body", &mut headers).unwrap(),
        0
    );
    assert_eq!(headers.len(), count);
    (
        headers,
        consumed,
        parser.body_separator().map(<[u8]>::to_vec),
    )
}

fn raw_records(headers: &[Header]) -> Vec<Vec<u8>> {
    headers.iter().map(|header| header.raw().to_vec()).collect()
}

#[test]
fn raw_fields_and_values_preserve_bytes() {
    let input = b"sUbJeCt \t: hello\r\n\tworld\nX-Raw: \xff\0\rz\r\n\r\nbody";
    let (headers, consumed, separator) = parse_chunks(input, 1);
    assert_eq!(headers.len(), 2);
    assert_eq!(&input[consumed..], b"body");
    assert_eq!(separator.as_deref(), Some(&b"\r\n"[..]));
    assert_eq!(headers[0].field(), Some("sUbJeCt"));
    assert_eq!(headers[0].raw_field(), b"sUbJeCt \t");
    assert_eq!(headers[0].raw_value(), b" hello\r\n\tworld\n");
    assert_eq!(headers[1].raw_value(), b" \xff\0\rz\r\n");
    assert!(!headers[0].is_invalid());
    assert!(!headers[1].is_invalid());
    assert!(headers[0].matches_field("SUBJECT"));
    assert!(!headers[0].matches_field("Subject "));
    assert!(!headers[0].matches_field("subje\u{00e7}t"));
}

#[test]
fn duplicate_fields_remain_ordered() {
    let input = b"Received: first\nSubject: x\nreceived: second\n\n";
    let (headers, _, _) = parse_chunks(input, 7);
    let received: Vec<_> = headers
        .iter()
        .filter(|header| header.matches_field("RECEIVED"))
        .map(Header::raw_value)
        .collect();
    assert_eq!(received, [b" first\n".as_slice(), b" second\n"]);
    assert_eq!(headers[1].field(), Some("Subject"));
}

#[test]
fn malformed_records_are_explicit_and_lossless() {
    let records: &[&[u8]] = &[
        b"No colon\n",
        b"Bad Name: value\n folded\n",
        b"Bad\0Name: value\n",
        b"Bad\x7fName: value\n",
        b"\xff: value\n",
        b"\r\r\n",
    ];
    let mut input = Vec::new();
    for record in records {
        input.extend_from_slice(record);
    }
    input.push(b'\n');
    for size in [1, 2, 7, input.len()] {
        let (headers, _, _) = parse_chunks(&input, size);
        assert_eq!(headers.len(), records.len());
        for (header, raw) in headers.iter().zip(records) {
            assert!(header.is_invalid());
            assert_eq!(header.field(), None);
            assert!(!header.matches_field(""));
            assert_eq!(header.raw_field(), *raw);
            assert!(header.raw_value().is_empty());
            assert_eq!(header.raw(), *raw);
        }
    }
}

#[test]
fn field_character_classification_matches_mimekit() {
    for byte in 0..=255 {
        // An initial nonblank byte ensures controls and blanks are tested as
        // part of a field rather than as line separators or continuation starts.
        if matches!(byte, b'\r' | b'\n' | b':') {
            continue;
        }
        let input = [b'X', byte, b'Y', b':', b'v'];
        let (headers, _, _) = parse_chunks(&input, 1);
        assert_eq!(headers.len(), 1, "byte {byte}");
        let valid = (33..=126).contains(&byte);
        assert_eq!(!headers[0].is_invalid(), valid, "byte {byte}");
        if valid {
            assert_eq!(headers[0].field().unwrap().as_bytes(), &input[..3]);
        }
    }
}

#[test]
fn empty_names_and_additional_colons_match_mimekit() {
    let (headers, _, _) = parse_chunks(b": value\n:: another\nX: a:b\n\n", 1);
    assert_eq!(headers.len(), 3);
    assert_eq!(headers[0].field(), Some(""));
    assert_eq!(headers[0].raw_field(), b"");
    assert_eq!(headers[0].raw_value(), b" value\n");
    assert!(!headers[0].is_invalid());
    assert!(headers[0].matches_field(""));
    assert_eq!(headers[1].field(), Some(""));
    assert_eq!(headers[1].raw_value(), b": another\n");
    assert_eq!(headers[2].raw_value(), b" a:b\n");
}

#[test]
fn whitespace_only_lines_are_continuations_not_separators() {
    let input = b"Subject: a\r\n \t\r\n\t\n b\n\nbody";
    let (headers, consumed, separator) = parse_chunks(input, 1);
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].raw(), b"Subject: a\r\n \t\r\n\t\n b\n");
    assert_eq!(separator.as_deref(), Some(&b"\n"[..]));
    assert_eq!(&input[consumed..], b"body");

    let (headers, _, _) = parse_chunks(b" orphan\n continuation\n\n", 2);
    assert_eq!(headers.len(), 1);
    assert!(headers[0].is_invalid());
    assert_eq!(headers[0].raw(), b" orphan\n continuation\n");
}

#[test]
fn source_carriage_return_only_regressions() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"A: b\n\r\r\nC: d\n\n", b"\r\r\n"),
        (b"A: b\r\n\r\r\r\nC: d\r\n\r\n", b"\r\r\r\n"),
        (b"A: b\n\r\r\n x\r\nC: d\n\n", b"\r\r\n x\r\n"),
        (b"A: b\n\r\r\n x\nC: d\n\n", b"\r\r\n x\n"),
    ];
    for &(input, invalid) in cases {
        for size in 1..=input.len() {
            let (headers, consumed, separator) = parse_chunks(input, size);
            assert_eq!(headers.len(), 3);
            assert!(headers[1].is_invalid());
            assert_eq!(headers[1].raw(), invalid);
            assert_eq!(consumed, input.len());
            let mut restored = raw_records(&headers).concat();
            restored.extend_from_slice(separator.as_deref().unwrap());
            assert_eq!(restored, input);
        }
    }
}

#[test]
fn every_split_preserves_records_body_boundary_and_clone_state() {
    let cases: &[&[u8]] = &[
        b"Subject: one\r\n\tcontinued\r\nX: two\n\r\nbody\r\nX: body",
        b"A: b\n\r\r\n x\r\nC: d\n\nbody",
        b"X: \xff\nY: missing final newline\r",
        b"Header-1: value 1\r\nJunk",
        b"Header-1: value 1\r\nJunk\r",
        b"Header-1: value 1\r\nJunk\r\n",
        b": value\n:: value\n\n",
        b"\r",
        b"\r\nbody",
        b"\nbody",
        b"",
    ];
    for &input in cases {
        let (expected, expected_count, expected_separator) =
            parse_chunks(input, input.len().max(1));
        for split in 0..=input.len() {
            let mut parser = HeaderParser::new();
            let mut headers = Vec::new();
            let first = parser.parse(&input[..split], &mut headers).unwrap();
            let mut cloned = parser.clone();
            let mut cloned_headers = headers.clone();

            let second = parser.parse(&input[split..], &mut headers).unwrap();
            parser.finish(&mut headers).unwrap();
            assert_eq!(headers, expected, "split {split}, input {input:?}");
            assert_eq!(first + second, expected_count);
            assert_eq!(parser.body_separator(), expected_separator.as_deref());

            let mut cloned_count = first;
            for byte in &input[split..] {
                cloned_count += cloned.parse(&[*byte], &mut cloned_headers).unwrap();
            }
            cloned.finish(&mut cloned_headers).unwrap();
            assert_eq!(cloned_headers, expected);
            assert_eq!(cloned_count, expected_count);
            assert_eq!(cloned.body_separator(), expected_separator.as_deref());
        }
    }
}

#[test]
fn finish_preserves_truncation_and_recognizes_final_separator_cr() {
    type EofCase<'a> = (&'a [u8], &'a [u8], Option<&'a [u8]>);
    let cases: &[EofCase<'_>] = &[
        (b"X: value", b"X: value", None),
        (b"X: value\r", b"X: value\r", None),
        (b"X: value\r\n", b"X: value\r\n", None),
        (b"X: value\r\n\r", b"X: value\r\n", Some(b"\r")),
        (b"X: value\n \r", b"X: value\n \r", None),
        (b"X:", b"X:", None),
        (b"X: value\nJunk", b"X: value\nJunk", None),
    ];
    for &(input, expected, separator) in cases {
        let (headers, count, actual_separator) = parse_chunks(input, 1);
        assert_eq!(raw_records(&headers).concat(), expected);
        assert_eq!(count, input.len());
        assert_eq!(actual_separator.as_deref(), separator);
    }
}

#[test]
fn empty_blocks_and_empty_input_have_distinct_completion() {
    for input in [b"\r\nbody".as_slice(), b"\nbody", b"\r", b""] {
        let (headers, consumed, separator) = parse_chunks(input, 1);
        assert!(headers.is_empty());
        if input.is_empty() {
            assert_eq!(separator, None);
        } else {
            assert_eq!(separator.as_deref(), Some(&input[..consumed]));
        }
    }
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    assert_eq!(parser.parse(b"", &mut headers).unwrap(), 0);
    assert!(!parser.is_finished());
    parser.parse(b"X: a\r", &mut headers).unwrap();
    assert_eq!(parser.parse(b"", &mut headers).unwrap(), 0);
    assert!(headers.is_empty());
    assert!(!parser.is_finished());
    parser.parse(b"\n\n", &mut headers).unwrap();
    assert_eq!(headers[0].raw(), b"X: a\r\n");
}

#[test]
fn appends_to_caller_output_and_emits_as_soon_as_folding_is_resolved() {
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    parser.parse(b"First: a\n", &mut headers).unwrap();
    assert!(headers.is_empty());
    parser.parse(b" ", &mut headers).unwrap();
    assert!(headers.is_empty());
    parser.parse(b"fold\nS", &mut headers).unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].raw(), b"First: a\n fold\n");
    parser.parse(b"econd: b\n\nbody", &mut headers).unwrap();
    assert_eq!(headers.len(), 2);
    let first_block = headers.clone();
    parser.reset();
    parser.parse(b"Third: c", &mut headers).unwrap();
    parser.finish(&mut headers).unwrap();
    assert_eq!(&headers[..2], first_block);
    assert_eq!(headers[2].raw(), b"Third: c");
}

#[test]
fn reset_discards_partial_lines_separators_and_completion() {
    for partial in [
        b"X: partial".as_slice(),
        b"X: a\r\n\r",
        b"X: a\r\n\r\n",
        b"Bad header\n",
    ] {
        let mut parser = HeaderParser::new();
        let mut headers = Vec::new();
        parser.parse(partial, &mut headers).unwrap();
        let existing = headers.clone();
        parser.reset();
        assert!(!parser.is_finished());
        assert_eq!(parser.body_separator(), None);
        parser.parse(b"Y: fresh\n\n", &mut headers).unwrap();
        assert_eq!(&headers[..existing.len()], existing);
        assert_eq!(headers.last().unwrap().raw(), b"Y: fresh\n");
    }
}

fn limited(limit: usize) -> HeaderParser {
    HeaderParser::with_max_header_length(NonZeroUsize::new(limit).unwrap())
}

#[test]
fn record_limit_counts_folds_and_newlines_but_not_separator_or_body() {
    let input = b"X: a\r\n b\n";
    for size in 1..=input.len() {
        let mut parser = limited(input.len());
        let mut headers = Vec::new();
        for chunk in input.chunks(size) {
            parser.parse(chunk, &mut headers).unwrap();
        }
        assert_eq!(parser.parse(b"\r\nlarge body", &mut headers).unwrap(), 2);
        assert_eq!(headers[0].raw(), input);

        let mut parser = limited(input.len() - 1);
        let mut headers = Vec::new();
        let error = input
            .chunks(size)
            .find_map(|chunk| parser.parse(chunk, &mut headers).err())
            .expect("one byte over the limit must fail");
        assert_eq!(error.limit(), input.len() - 1);
        assert!(headers.is_empty());
    }
}

#[test]
fn record_limit_is_per_header_not_per_block() {
    let mut parser = limited(3);
    let mut headers = Vec::new();
    let input = b"A:\nB:\nC:\n\nbody";
    assert_eq!(parser.parse(input, &mut headers).unwrap(), 10);
    assert_eq!(headers.len(), 3);
    assert!(headers.iter().all(|header| !header.is_invalid()));
    assert_eq!(parser.max_header_length(), 3);
    assert_eq!(HeaderParser::default().max_header_length(), 16 * 1024);
}

#[test]
fn limit_errors_latch_preserve_output_and_reset_retains_limit() {
    let mut parser = limited(5);
    let mut headers = Vec::new();
    let error = parser
        .parse(b"A: b\nB: too long\n\nbody", &mut headers)
        .unwrap_err();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].raw(), b"A: b\n");
    assert_eq!(error.limit(), 5);
    assert_eq!(
        error.to_string(),
        "header exceeds the maximum length of 5 bytes"
    );
    let _: &dyn std::error::Error = &error;
    assert!(!parser.is_finished());
    for input in [b"".as_slice(), b"\n", b"New: value"] {
        assert_eq!(parser.parse(input, &mut headers), Err(error));
    }
    assert_eq!(parser.finish(&mut headers), Err(error));
    assert_eq!(headers.len(), 1);
    let mut clone = parser.clone();
    assert_eq!(clone.finish(&mut headers), Err(error));

    parser.reset();
    assert_eq!(parser.max_header_length(), 5);
    parser.parse(b"C: d\n\n", &mut headers).unwrap();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[1].raw(), b"C: d\n");
}

#[test]
fn minimum_limit_and_pending_carriage_return() {
    for separator in [b"\n".as_slice(), b"\r\n", b"\r"] {
        let mut parser = limited(1);
        let mut headers = Vec::new();
        for byte in separator {
            parser.parse(&[*byte], &mut headers).unwrap();
        }
        parser.finish(&mut headers).unwrap();
        assert!(headers.is_empty());
        assert_eq!(parser.body_separator(), Some(separator));
    }

    let mut parser = limited(1);
    let mut headers = Vec::new();
    parser.parse(b"\r", &mut headers).unwrap();
    let error = parser.parse(b"x", &mut headers).unwrap_err();
    assert_eq!(error.limit(), 1);
    assert!(headers.is_empty());
    assert_eq!(parser.body_separator(), None);

    let mut parser = limited(1);
    parser.parse(b":", &mut headers).unwrap();
    parser.finish(&mut headers).unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].field(), Some(""));
}

#[test]
fn default_limit_is_enforced_at_exact_boundary() {
    let limit = HeaderParser::new().max_header_length();
    assert_eq!(limit, 16 * 1024);
    assert_eq!(
        mimecraft::MimeEntityParseOptions::default().max_header_length,
        limit
    );
    assert_eq!(
        mimecraft::DeliveryStatusParseOptions::default().max_header_length,
        limit
    );
    let mut input = b"X: ".to_vec();
    input.resize(limit, b'a');
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    assert_eq!(parser.parse(&input, &mut headers).unwrap(), limit);
    let mut too_long = parser.clone();
    parser.finish(&mut headers).unwrap();
    assert_eq!(headers[0].raw().len(), limit);
    assert_eq!(
        too_long.parse(b"a", &mut Vec::new()).unwrap_err().limit(),
        limit
    );
}

// Independent whole-line model for grouping and byte preservation. It has no
// pending-CR state and never processes partial lines.
fn reference(input: &[u8]) -> (Vec<Vec<u8>>, usize, Option<Vec<u8>>) {
    let mut records: Vec<Vec<u8>> = Vec::new();
    let mut consumed = 0;
    let mut separator = None;
    for line in input.split_inclusive(|&byte| byte == b'\n') {
        consumed += line.len();
        if matches!(line, b"\n" | b"\r\n" | b"\r") {
            separator = Some(line.to_vec());
            break;
        }
        if matches!(line.first(), Some(b' ' | b'\t')) && !records.is_empty() {
            records.last_mut().unwrap().extend_from_slice(line);
        } else {
            records.push(line.to_vec());
        }
    }
    (records, consumed, separator)
}

#[test]
fn arbitrary_bytes_match_whole_line_reference() {
    let alphabet = b"ABxy09: \t\r\n\0\x7f\xff";
    let mut state = 0x1234_5678u32;
    for length in 0..256 {
        let mut input = Vec::new();
        for _ in 0..length {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            input.push(alphabet[(state >> 16) as usize % alphabet.len()]);
        }
        let (expected, expected_count, expected_separator) = reference(&input);
        for size in [1, 2, 7, 31, 4096] {
            let (headers, consumed, separator) = parse_chunks(&input, size);
            assert_eq!(raw_records(&headers), expected, "input {input:?}");
            assert_eq!(consumed, expected_count);
            assert_eq!(separator, expected_separator);
            let mut restored = raw_records(&headers).concat();
            restored.extend_from_slice(separator.as_deref().unwrap_or_default());
            assert_eq!(restored, input[..consumed]);
        }
    }
}

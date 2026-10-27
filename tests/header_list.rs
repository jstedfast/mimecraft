// Collection behavior adapted from MimeKit's HeaderListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::iter::FusedIterator;

use mimecraft::{ContentType, Header, HeaderId, HeaderList, HeaderListEditError, HeaderParser};

fn parse(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    assert_eq!(parser.parse(raw, &mut records).unwrap(), raw.len());
    parser.finish(&mut records).unwrap();
    assert_eq!(parser.body_separator(), None);
    records.into()
}

fn header(raw: &[u8]) -> Header {
    let mut records = parse(raw).into_vec();
    assert_eq!(records.len(), 1);
    records.pop().unwrap()
}

fn raw_records(list: &HeaderList) -> Vec<&[u8]> {
    list.iter().map(Header::raw).collect()
}

fn assert_lookup(list: &HeaderList, expected: &[Header], field: &str) {
    let matches: Vec<_> = expected
        .iter()
        .enumerate()
        .filter(|(_, header)| {
            header
                .field()
                .is_some_and(|name| name.eq_ignore_ascii_case(field))
        })
        .collect();
    assert_eq!(list.contains(field), !matches.is_empty());
    assert_eq!(list.index_of(field), matches.first().map(|&(i, _)| i));
    assert_eq!(list.last_index_of(field), matches.last().map(|&(i, _)| i));
    assert_eq!(list.get(field), matches.first().map(|&(_, h)| h));
    assert_eq!(
        list.get_all(field).collect::<Vec<_>>(),
        matches.iter().map(|&(_, h)| h).collect::<Vec<_>>()
    );
}

#[test]
fn empty_lists_and_first_last_lookup() {
    let mut list = HeaderList::new();
    assert_eq!(list, HeaderList::default());
    assert!(list.is_empty());
    assert_lookup(&list, &[], "Subject");
    assert_eq!(list.remove("Subject"), None);
    assert_eq!(list.remove_all("Subject"), 0);
    assert_eq!(list.iter().count(), 0);
    assert_eq!(HeaderList::with_capacity(8).len(), 0);
    assert!(HeaderList::with_capacity(8).into_vec().capacity() >= 8);

    let list = parse(b"From: sender\r\nTo: first\r\nSubject \t: hello\r\ntO: second\r\n");
    assert_eq!(list.len(), 4);
    assert_eq!(list.index_of("TO"), Some(1));
    assert_eq!(list.last_index_of("to"), Some(3));
    assert_eq!(list.get("To").unwrap().raw_value(), b" first\r\n");
    let subject = list.get("SUBJECT").unwrap();
    assert_eq!(subject.field(), Some("Subject"));
    assert_eq!(subject.raw_field(), b"Subject \t");
    assert_eq!(subject.id(), HeaderId::Subject);
    for field in ["Subject ", "Subject:", "Missing", "", "\u{e9}"] {
        assert_lookup(&list, &[], field);
    }
}

#[test]
fn long_field_names_preserve_case_insensitive_lookup_after_edits() {
    for length in [1, 7, 8, 15, 16, 31, 32, 33, 63, 64, 65, 127, 128, 129, 1024] {
        let upper = "A".repeat(length);
        let lower = upper.to_ascii_lowercase();
        let mixed: String = (0..length)
            .map(|index| if index % 2 == 0 { 'A' } else { 'a' })
            .collect();
        let mut list = parse(format!("{upper}: first\r\n{lower}: second\r\n").as_bytes());
        for field in [&upper, &lower, &mixed] {
            assert_eq!(list.index_of(field), Some(0), "{length}");
            assert_eq!(list.get(field).unwrap().raw_value(), b" first\r\n");
        }
        assert_eq!(list.remove(&mixed).unwrap().raw_value(), b" first\r\n");
        assert_eq!(list.get(&upper).unwrap().raw_value(), b" second\r\n");
        list.replace(0, header(format!("{mixed}: replaced\r\n").as_bytes()))
            .unwrap();
        assert_eq!(list.get(&lower).unwrap().raw_value(), b" replaced\r\n");
        assert_eq!(list, list.clone().into_vec().into());
        assert_eq!(list.remove_all(&upper), 1);
        assert!(!list.contains(&lower));
        list.add(header(format!("{mixed}: reinserted\r\n").as_bytes()));
        assert!(list.contains(&lower));
        for suffix in ["A", "-", "\u{e9}", "\0"] {
            assert!(!list.contains(&format!("{lower}{suffix}")));
        }
    }
}

#[test]
fn insert_and_append_preserve_duplicates_and_change_first_lookup() {
    let original = parse(b"Received: first\r\nSubject: hello\r\nreceived: last\r\n");
    for index in 0..=original.len() {
        let new = header(b"RECEIVED: inserted\r\n");
        let mut expected = original.clone().into_vec();
        expected.insert(index, new.clone());
        let mut list = original.clone();
        list.insert(index, new).unwrap();
        assert_eq!(list.as_slice(), expected);
        assert_lookup(&list, &expected, "received");
        assert_lookup(&list, &expected, "Subject");
    }

    let mut list = original;
    list.add(header(b"Received: appended\r\n"));
    assert_eq!(list.get_all("RECEIVED").count(), 3);
    assert_eq!(list.get("Received").unwrap().raw_value(), b" first\r\n");
    assert_eq!(list.last_index_of("received"), Some(3));
    list.clear();
    assert!(list.is_empty());
    list.insert(0, header(b"Received: empty list\r\n")).unwrap();
    assert_eq!(
        list.get("received").unwrap().raw_value(),
        b" empty list\r\n"
    );
}

#[test]
fn removing_first_promotes_next_duplicate() {
    let mut list = parse(
        b"From: sender@localhost\r\n\
To: first@localhost\r\n\
To: second@localhost\r\n\
To: third@localhost\r\n\
To: fourth@localhost\r\n\
Cc: carbon.copy@localhost\r\n",
    );
    assert_eq!(
        list.remove("cc").unwrap().raw_value(),
        b" carbon.copy@localhost\r\n"
    );
    assert_eq!(list.remove("Cc"), None);

    for value in ["first", "second", "third"] {
        let expected = format!(" {value}@localhost\r\n");
        assert_eq!(list.get("TO").unwrap().raw_value(), expected.as_bytes());
        // Removing the first occurrence must expose the next, not hide the field.
        let removed = if value == "third" {
            list.remove_at(list.index_of("to").unwrap()).unwrap()
        } else {
            list.remove("to").unwrap()
        };
        assert_eq!(removed.raw_value(), expected.as_bytes());
    }
    assert_eq!(
        list.get("To").unwrap().raw_value(),
        b" fourth@localhost\r\n"
    );
    assert_eq!(list.index_of("To"), Some(1));
    assert_eq!(list.last_index_of("To"), Some(1));
    assert_eq!(list.remove_all("TO"), 1);
    assert!(!list.contains("to"));
    assert_eq!(list.len(), 1);
}

#[test]
fn remove_all_keeps_nonmatching_bytes_and_order() {
    let mut list = parse(
        b"Received: first\r\nX-A: \xff\r\nreceived: second\n\
\r\r\nX-B: folded\r\n\tvalue\r\nRECEIVED: last\r\nBroken",
    );
    assert_eq!(list.remove_all("rEcEiVeD"), 3);
    assert_eq!(
        raw_records(&list),
        [
            &b"X-A: \xff\r\n"[..],
            &b"\r\r\n"[..],
            &b"X-B: folded\r\n\tvalue\r\n"[..],
            &b"Broken"[..],
        ]
    );
    let before = list.clone();
    assert_eq!(list.remove_all("Received"), 0);
    assert_eq!(list, before);
    assert_eq!(list.remove_all(""), 0);
    assert_eq!(list, before);
}

#[test]
fn replace_all_combines_recipients_at_the_first_position() {
    let mut list = parse(
        b"From: sender@localhost\r\n\
To: first@localhost\r\n\
X-Trace: between\r\n\
tO: second@localhost\r\n\
TO: third@localhost\r\n\
Cc: carbon.copy@localhost\r\n",
    );
    let replacement = header(b"to: first@localhost, second@localhost, third@localhost\r\n");
    let old = list.replace_all(replacement.clone()).unwrap().unwrap();
    assert_eq!(old.raw(), b"To: first@localhost\r\n");
    assert_eq!(list.len(), 4);
    assert_eq!(list.index_of("To"), Some(1));
    assert_eq!(list.last_index_of("To"), Some(1));
    assert_eq!(list.get("To"), Some(&replacement));
    assert_eq!(
        list.iter().map(Header::field).collect::<Vec<_>>(),
        [Some("From"), Some("to"), Some("X-Trace"), Some("Cc")]
    );
    assert_eq!(list.get("X-Trace").unwrap().raw(), b"X-Trace: between\r\n");
    assert_eq!(
        list.replace_all(replacement.clone()).unwrap(),
        Some(replacement)
    );
    assert_eq!(list.len(), 4);
}

#[test]
fn replace_all_handles_every_duplicate_position_and_missing_fields() {
    // Exercise adjacent, interleaved, all-matching, and absent fields, including
    // replacements at either end of the list. The first slot must never move.
    for len in 0..=6 {
        for mask in 0..1 << len {
            let original: HeaderList = (0..len)
                .map(|i| {
                    let name = if mask & (1 << i) != 0 { "X-A" } else { "X-B" };
                    header(format!("{name}: {i}\r\n").as_bytes())
                })
                .collect();
            let replacement = header(b"x-a: replacement\r\n");
            let first = original.iter().position(|h| h.field() == Some("X-A"));
            let previous = first.map(|i| original.as_slice()[i].clone());
            let mut expected: Vec<_> = original
                .iter()
                .enumerate()
                .filter_map(|(i, h)| {
                    if Some(i) == first {
                        Some(replacement.clone())
                    } else if h.field() == Some("X-A") {
                        None
                    } else {
                        Some(h.clone())
                    }
                })
                .collect();
            if first.is_none() {
                expected.push(replacement.clone());
            }
            let mut list = original;
            assert_eq!(list.replace_all(replacement).unwrap(), previous);
            assert_eq!(list.as_slice(), expected);
            assert_lookup(&list, &expected, "X-A");
            assert_lookup(&list, &expected, "x-b");
        }
    }
}

#[test]
fn positional_replacement_preserves_duplicates_and_updates_both_fields() {
    let original = parse(
        b"Received: first\r\nReceived: second\r\nReturn-Path: old\r\n\
received: third\r\nBroken\r\nX-Other: tail\r\n",
    );
    for index in 0..original.len() {
        for raw in [
            &b"RECEIVED: new\r\n"[..],
            &b"Return-Path: new\r\n"[..],
            &b"Subject: new\r\n"[..],
            &b"Malformed\r\n"[..],
        ] {
            let replacement = header(raw);
            let mut expected = original.clone().into_vec();
            let previous = std::mem::replace(&mut expected[index], replacement.clone());
            let mut list = original.clone();
            assert_eq!(list.replace(index, replacement).unwrap(), previous);
            assert_eq!(list.as_slice(), expected);
            for field in ["received", "RETURN-PATH", "Subject", "X-Other", "Malformed"] {
                assert_lookup(&list, &expected, field);
            }
        }
    }
}

#[test]
fn positional_removal_preserves_remaining_order() {
    let original = parse(b"Received: first\r\nBroken\r\nSubject: hello\r\nReceived: last\r\n");
    for index in 0..original.len() {
        let mut expected = original.clone().into_vec();
        let previous = expected.remove(index);
        let mut list = original.clone();
        assert_eq!(list.remove_at(index).unwrap(), previous);
        assert_eq!(list.as_slice(), expected);
        assert_lookup(&list, &expected, "Received");
        assert_lookup(&list, &expected, "Subject");
    }
}

#[test]
fn rejected_edits_leave_the_list_unchanged() {
    for mut list in [
        HeaderList::new(),
        parse(b"Subject: first\r\nSubject: second\r\n"),
    ] {
        let before = list.clone();
        for index in [list.len() + 1, usize::MAX] {
            let error = HeaderListEditError::IndexOutOfBounds {
                index,
                len: list.len(),
            };
            assert_eq!(list.insert(index, header(b"X-New: x\r\n")), Err(error));
            assert_eq!(list, before);
        }
        for index in [list.len(), list.len() + 1, usize::MAX] {
            let error = HeaderListEditError::IndexOutOfBounds {
                index,
                len: list.len(),
            };
            assert_eq!(list.replace(index, header(b"X-New: x\r\n")), Err(error));
            assert_eq!(list, before);
            assert_eq!(list.remove_at(index), Err(error));
            assert_eq!(list, before);
            assert_eq!(
                error.to_string(),
                format!(
                    "header index {index} is out of bounds for length {}",
                    list.len()
                )
            );
        }
        for raw in [
            &b"Broken\r\n"[..],
            &b"\r\r\n"[..],
            &b"Bad field: value\r\n"[..],
        ] {
            assert_eq!(
                list.replace_all(header(raw)),
                Err(HeaderListEditError::InvalidHeader)
            );
            assert_eq!(list, before);
        }
    }
    assert_eq!(
        HeaderListEditError::InvalidHeader.to_string(),
        "replacement header has no parsed field name"
    );
}

#[test]
fn malformed_records_can_be_added_and_inserted_without_repair() {
    let mut list = HeaderList::with_capacity(8);
    list.add(header(b"Broken\r\n"));
    list.insert(0, header(b"\r\r\n")).unwrap();
    list.add(header(b"X-End: unterminated"));
    assert_eq!(
        raw_records(&list),
        [
            &b"\r\r\n"[..],
            &b"Broken\r\n"[..],
            &b"X-End: unterminated"[..]
        ]
    );
    assert_eq!(list.iter().filter(|h| h.is_invalid()).count(), 2);
    assert_eq!(list.get_all("").count(), 0);
    list.clear();
    assert!(list.is_empty());
    assert!(list.into_vec().capacity() >= 8);
}

#[test]
fn unknown_names_empty_names_and_malformed_records_are_distinct() {
    let mut list = parse(
        b"X-A: first\r\nX-B: other\r\nBroken\r\n: empty first\r\n\
Unknown: literal\r\nx-a: second\r\n: empty second\r\n",
    );
    assert!(list.iter().all(|h| h.id() == HeaderId::Unknown));
    assert_eq!(list.get_all("X-A").count(), 2);
    assert_eq!(list.get_all("").count(), 2);
    assert_eq!(list.get("UNKNOWN").unwrap().raw_value(), b" literal\r\n");
    assert_eq!(list.get("Broken"), None);
    assert_eq!(list.get("Broken\r\n"), None);
    assert_eq!(list.get(""), Some(&list.as_slice()[3]));

    list.replace_all(header(b"X-A: new\r\n")).unwrap();
    assert_eq!(list.len(), 6);
    assert!(list.contains("X-B"));
    assert!(list.contains("Unknown"));
    let old = list
        .replace_all(header(b": replacement\r\n"))
        .unwrap()
        .unwrap();
    assert_eq!(old.raw(), b": empty first\r\n");
    assert_eq!(list.get_all("").count(), 1);
    assert_eq!(list.index_of(""), Some(3));
    assert_eq!(list.remove_all(""), 1);
    assert_eq!(list.as_slice()[2].raw(), b"Broken\r\n");
    assert!(list.as_slice()[2].is_invalid());
    assert_eq!(
        list.remove("unknown").unwrap().raw(),
        b"Unknown: literal\r\n"
    );
    assert_eq!(list.remove_at(2).unwrap().raw(), b"Broken\r\n");
    assert_eq!(list.len(), 2);
}

#[test]
fn iterators_conversions_and_clones_keep_all_records() {
    fn assert_fused<'a>(_: impl FusedIterator<Item = &'a Header>) {}

    let original = parse(b"X-A: first\r\nBroken\r\nx-a: second\r\nX-B: last\r\n");
    let records = original.clone().into_vec();
    let allocation = records.as_ptr();
    let mut list = HeaderList::from(records);
    assert_eq!(list.as_slice().as_ptr(), allocation);
    assert_eq!(list, original);
    assert_eq!((&list).into_iter().len(), 4);
    assert_eq!(
        (&list)
            .into_iter()
            .rev()
            .map(Header::raw)
            .collect::<Vec<_>>(),
        original.iter().rev().map(Header::raw).collect::<Vec<_>>()
    );
    let collected: HeaderList = list.clone().into_iter().collect();
    assert_eq!(collected, original);
    let mut extended = HeaderList::new();
    extended.extend(list.clone());
    extended.extend(HeaderList::new());
    assert_eq!(extended, original);
    list.remove_at(0).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(original.len(), 4);
    assert_eq!(collected, original);

    let mut matching = original.get_all("X-A");
    assert_eq!(matching.next_back().unwrap().raw_value(), b" second\r\n");
    assert_eq!(matching.next().unwrap().raw_value(), b" first\r\n");
    assert!(matching.next().is_none());
    assert!(matching.next_back().is_none());
    assert!(matching.next().is_none());
    assert_fused(original.get_all("X-A"));
}

#[test]
fn parser_batches_preserve_bytes_and_keep_the_body_boundary_separate() {
    let raw = b"Subject: =?utf-8?q?caf=C3=A9?=\r\n\tfolded\r\n\
X-Raw \t: \xff\0\n\r\r\nBroken\r\nx-raw: last\r\n";
    let mut input = raw.to_vec();
    input.extend_from_slice(b"\r\nbody");
    let expected = parse(raw);
    for chunk_size in [1, 2, 7, input.len()] {
        let mut parser = HeaderParser::new();
        let mut list = HeaderList::with_capacity(expected.len());
        let mut batch = Vec::new();
        let mut consumed = 0;
        for chunk in input.chunks(chunk_size) {
            consumed += parser.parse(chunk, &mut batch).unwrap();
            list.extend(batch.drain(..));
            if parser.is_finished() {
                break;
            }
        }
        parser.finish(&mut batch).unwrap();
        list.extend(batch);
        assert_eq!(list, expected);
        assert_eq!(parser.body_separator(), Some(&b"\r\n"[..]));
        assert_eq!(&input[consumed..], b"body");
        assert_eq!(raw_records(&list).concat(), raw);
    }
    let truncated = b"Subject: no final newline\r\nX-End: \xff";
    assert_eq!(raw_records(&parse(truncated)).concat(), truncated);
}

#[test]
fn typed_content_headers_can_replace_parsed_raw_records() {
    let mut list = parse(
        b"Subject: keep me\r\nContent-Type: text/plain\r\n\
X-Trace: unchanged\r\ncontent-type: application/octet-stream\r\n",
    );
    let mut content_type = ContentType::new("text", "html").unwrap();
    content_type
        .parameters_mut()
        .set("name", "caf\u{e9}.html")
        .unwrap();
    let formatted = content_type.to_header_string().unwrap();
    let previous = list
        .replace_all(header(formatted.as_bytes()))
        .unwrap()
        .unwrap();
    assert_eq!(previous.raw(), b"Content-Type: text/plain\r\n");
    assert_eq!(list.get_all("Content-Type").count(), 1);
    assert_eq!(list.index_of("Content-Type"), Some(1));
    let raw = list.get("content-type").unwrap();
    assert_eq!(raw.raw(), formatted.as_bytes());
    assert_eq!(raw.id(), HeaderId::ContentType);
    let mut parameter = content_type.parameters().as_slice()[0].clone();
    parameter.set_encoding_method(Some(mimecraft::ParameterEncodingMethod::Rfc2231));
    parameter.set_charset(Some(mimecraft::charsets::Charset::LATIN1));
    content_type.parameters_mut().replace(0, parameter).unwrap();
    assert_eq!(ContentType::parse(raw.raw_value()).unwrap(), content_type);
    assert_eq!(list.get("Subject").unwrap().raw(), b"Subject: keep me\r\n");
    assert_eq!(
        list.get("X-Trace").unwrap().raw(),
        b"X-Trace: unchanged\r\n"
    );
}

#[test]
fn indexed_names_fold_only_ascii_letters() {
    let long_name = format!("X-{}", "MixedCase".repeat(128));
    let names = ["X-@", "X-`", "X-[", "X-{", "K", "Subject", &long_name];
    let mut list = HeaderList::with_capacity(1);
    for (index, name) in names.iter().enumerate() {
        list.add(header(format!("{name}: {index}\r\n").as_bytes()));
    }
    for (index, name) in names.iter().enumerate() {
        for query in [
            name.to_string(),
            name.to_ascii_lowercase(),
            name.to_ascii_uppercase(),
        ] {
            assert_eq!(list.index_of(&query), Some(index));
            assert_eq!(list.get(&query), Some(&list.as_slice()[index]));
        }
    }
    // ASCII case folding must not alias punctuation or Unicode lookalikes.
    for query in ["\u{212a}", "\u{17f}ubject", "Subject ", "Subject:"] {
        assert!(!list.contains(query));
    }
}

#[test]
fn dictionary_growth_keeps_first_occurrences() {
    let mut list = HeaderList::with_capacity(1);
    let mut expected = Vec::new();
    for index in 0..256 {
        let record = header(format!("X-Field-{index}: first\r\n").as_bytes());
        list.add(record.clone());
        expected.push(record);
    }
    list.extend((0..256).map(|index| header(format!("x-field-{index}: second\r\n").as_bytes())));
    for (index, record) in expected.iter().enumerate() {
        assert_eq!(list.index_of(&format!("X-FIELD-{index}")), Some(index));
        assert_eq!(list.get(&format!("x-field-{index}")), Some(record));
    }
    for (index, record) in expected.iter().enumerate() {
        let name = format!("x-field-{index}");
        assert_eq!(list.remove(&name), Some(record.clone()));
        assert_eq!(list.get(&name).unwrap().raw_value(), b" second\r\n");
    }
    for index in 0..256 {
        assert_eq!(list.index_of(&format!("X-Field-{index}")), Some(index));
    }
    list.clear();
    for index in 0..256 {
        assert_eq!(list.get(&format!("X-Field-{index}")), None);
    }
    list.add(header(b"x-FIELD-200: reused\r\n"));
    assert_eq!(list.index_of("X-Field-200"), Some(0));
}

#[test]
fn mixed_edits_keep_the_index_equal_to_a_linear_model() {
    fn choose(state: &mut u64, count: usize) -> usize {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((*state >> 32) as usize) % count
    }

    let pool = parse(
        b"Subject: first\r\nsubject: second\r\nReceived: first\r\nreceived: second\r\n\
X-A: first\r\nx-a: second\r\nX-B: b\r\nX-C: c\r\nUnknown: literal\r\n\
: empty\r\nBroken\r\n\r\r\n",
    );
    let queries = [
        "SUBJECT", "received", "X-A", "x-b", "X-C", "Unknown", "", "Broken", "Missing",
    ];
    let mut list = pool.clone();
    let mut expected = pool.clone().into_vec();
    let mut state = 0x81e5_d5d9_219b_e371u64;

    for step in 0..1000 {
        let operation = choose(&mut state, 10);
        let replacement = pool.as_slice()[choose(&mut state, pool.len())].clone();
        let query = queries[choose(&mut state, queries.len())];
        match operation {
            0 => {
                list.add(replacement.clone());
                expected.push(replacement);
            }
            1 => {
                let index = choose(&mut state, expected.len() + 1);
                list.insert(index, replacement.clone()).unwrap();
                expected.insert(index, replacement);
            }
            2 if !expected.is_empty() => {
                let index = choose(&mut state, expected.len());
                let previous = std::mem::replace(&mut expected[index], replacement.clone());
                assert_eq!(list.replace(index, replacement).unwrap(), previous);
            }
            3 if !expected.is_empty() => {
                let index = choose(&mut state, expected.len());
                assert_eq!(list.remove_at(index).unwrap(), expected.remove(index));
            }
            4 => {
                let first = expected.iter().position(|h| h.matches_field(query));
                let previous = first.map(|index| expected.remove(index));
                assert_eq!(list.remove(query), previous);
            }
            5 => {
                if let Some(field) = replacement.field() {
                    let first = expected.iter().position(|h| h.matches_field(field));
                    let previous = first.map(|index| expected[index].clone());
                    expected.retain(|h| !h.matches_field(field));
                    expected.insert(first.unwrap_or(expected.len()), replacement.clone());
                    assert_eq!(list.replace_all(replacement).unwrap(), previous);
                } else {
                    assert_eq!(
                        list.replace_all(replacement),
                        Err(HeaderListEditError::InvalidHeader)
                    );
                }
            }
            6 => {
                let before = expected.len();
                expected.retain(|h| !h.matches_field(query));
                assert_eq!(list.remove_all(query), before - expected.len());
            }
            7 => {
                let batch = [
                    replacement,
                    pool.as_slice()[choose(&mut state, pool.len())].clone(),
                ];
                list.extend(batch.iter().cloned());
                expected.extend(batch);
            }
            8 => {
                let mut copy = list.clone();
                copy.clear();
                for query in queries {
                    assert_eq!(copy.get(query), None);
                }
                list = list.into_vec().into_iter().collect();
            }
            9 => {
                list.clear();
                expected.clear();
            }
            _ => {
                assert_eq!(
                    list.replace(0, replacement),
                    Err(HeaderListEditError::IndexOutOfBounds { index: 0, len: 0 })
                );
            }
        }
        assert_eq!(list.as_slice(), expected, "step {step}");
        assert_eq!(list, HeaderList::from(expected.clone()), "step {step}");
        for query in queries {
            assert_lookup(&list, &expected, query);
        }
    }
}

#[test]
fn interrupted_extension_leaves_the_completed_prefix_indexed() {
    let mut list = parse(b"Subject: original\r\n");
    let incoming = (0..4).map(|index| {
        assert_ne!(index, 2, "iterator interrupted");
        header(format!("X-{index}: appended\r\n").as_bytes())
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list.extend(incoming)));
    assert!(result.is_err());
    let expected = parse(b"Subject: original\r\nX-0: appended\r\nX-1: appended\r\n");
    assert_eq!(list, expected);
    for query in ["SUBJECT", "x-0", "x-1", "x-2"] {
        assert_lookup(&list, expected.as_slice(), query);
    }
}

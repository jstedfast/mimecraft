// Adapted from MimeKit's Header.Offset and MimeParser header-offset tracking.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::BufReader;

use mimecraft::{Header, HeaderList, HeaderParser, MimeEntity, MimeMessage};

#[test]
fn authored_records_have_no_offset_and_clones_keep_parsed_offsets() {
    let raw = b"X: one\r\nSubject: longer\r\n\r\n";
    let mut headers = HeaderList::load(&mut raw.as_slice()).unwrap();
    let subject = headers.get("Subject").unwrap().clone();
    assert_eq!(subject.offset(), Some(8));
    assert_eq!(subject, Header::parse(b"Subject: longer\r\n").unwrap());
    assert_eq!(
        Header::parse(b"Subject: longer\r\n").unwrap().offset(),
        None
    );
    assert_eq!(
        Header::from_unstructured("Subject", "new")
            .unwrap()
            .offset(),
        None
    );
    assert_eq!(
        headers
            .replace_all(Header::parse(b"Subject: new").unwrap())
            .unwrap()
            .unwrap()
            .offset(),
        Some(8),
    );
    assert_eq!(headers.get("Subject").unwrap().offset(), None);
    headers
        .replace(0, Header::parse(b"X: new").unwrap())
        .unwrap();
    assert_eq!(headers.as_slice()[0].offset(), None);
    headers.replace(0, subject).unwrap();
    assert_eq!(headers.as_slice()[0].offset(), Some(8));

    let mut parsed = Vec::new();
    let mut parser = HeaderParser::new();
    parser.parse(raw, &mut parsed).unwrap();
    assert!(parsed.iter().all(|header| header.offset().is_none()));
}

#[test]
fn offsets_count_original_bytes_folds_and_malformed_records_at_every_buffer_size() {
    let raw = b"X: a\r\nLonger: b\n\tfold\r\nBroken\xff\n\r\r\nSubject: tail\n\nbody";
    for capacity in 1..=raw.len() {
        let mut reader = BufReader::with_capacity(capacity, raw.as_slice());
        let headers = HeaderList::load(&mut reader).unwrap();
        assert_eq!(headers.get("Longer").unwrap().offset(), Some(6));
        let mut offset = 0;
        for header in &headers {
            assert_eq!(header.offset(), Some(offset));
            assert_eq!(
                header.raw(),
                &raw[offset as usize..offset as usize + header.raw().len()]
            );
            offset += header.raw().len() as u64;
        }
        assert_eq!(&raw[offset as usize..], b"\nbody");
    }
}

#[test]
fn buffered_prefixes_clones_and_reset_keep_stream_offset_accounting() {
    let raw = b"X: a\nLonger: b\n\tfold\nLast: c\n\nbody";
    for split in 0..raw.len() {
        let mut parser = HeaderParser::new();
        let mut records = Vec::new();
        parser.parse(&raw[..split], &mut records).unwrap();
        let already_emitted = records.len();
        let mut cloned = parser.clone();
        let mut cloned_records = records.clone();
        parser.read_from(&mut &raw[split..], &mut records).unwrap();
        cloned
            .read_from(&mut &raw[split..], &mut cloned_records)
            .unwrap();
        let mut offset = 0;
        for (index, header) in records.iter().enumerate() {
            let expected = (index >= already_emitted).then_some(offset);
            assert_eq!(header.offset(), expected);
            assert_eq!(cloned_records[index].offset(), expected);
            offset += header.raw().len() as u64;
        }
        parser.reset();
        records.clear();
        parser
            .read_from(&mut &b"Reset: value\n\n"[..], &mut records)
            .unwrap();
        assert_eq!(records[0].offset(), Some(0));
    }
}

#[test]
fn truncated_stream_records_keep_offsets_without_synthetic_newlines() {
    for raw in [
        b"First: a\r\nLast: tail".as_slice(),
        b"First: a\r\nLast: tail\r",
    ] {
        let headers = HeaderList::load(&mut &raw[..]).unwrap();
        assert_eq!(headers.get("Last").unwrap().offset(), Some(10));
        assert_eq!(headers.get("Last").unwrap().raw(), &raw[10..]);
    }
}

#[test]
fn nested_entities_and_messages_share_the_original_input_stream_offsets() {
    let raw = "X-Top: head\nContent-Type: multipart/mixed; boundary=outer\nX-Tail: end\n\npreamble\n--outer\nContent-Type: message/rfc822\nX-Wrapper: yes\n\nX-Inside: value\nContent-Type: multipart/mixed; boundary=inner\nSubject: nested\n\ninner preamble\n--inner\nX-Leaf: child\nContent-Type: text/plain\n\nbody\n--inner--\nepilogue\n--outer--\ntail";
    for newline in ["\n", "\r\n"] {
        let input = raw.replace('\n', newline);
        for capacity in [1, 2, 7, input.len()] {
            let mut reader = BufReader::with_capacity(capacity, input.as_bytes());
            let message = MimeMessage::load(&mut reader).unwrap();
            let mut count = check_offsets(message.headers(), input.as_bytes());
            let mut stack = vec![message.body().unwrap()];
            while let Some(entity) = stack.pop() {
                count += check_offsets(entity.headers(), input.as_bytes());
                match entity {
                    MimeEntity::Part(_) => {}
                    MimeEntity::Multipart(part) => stack.extend(part.children()),
                    MimeEntity::Message(part) => {
                        let inner = part.message().unwrap();
                        count += check_offsets(inner.headers(), input.as_bytes());
                        stack.extend(inner.body());
                    }
                }
            }
            assert_eq!(count, 10);
        }
    }
}

fn check_offsets(headers: &HeaderList, input: &[u8]) -> usize {
    for header in headers {
        let expected = input
            .windows(header.raw().len())
            .position(|bytes| bytes == header.raw())
            .unwrap();
        assert_eq!(header.offset(), Some(expected as u64), "{header:?}");
    }
    headers.len()
}

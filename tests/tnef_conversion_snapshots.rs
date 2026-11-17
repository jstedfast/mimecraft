// Adapted from MimeKit's TnefConversionSnapshotTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::BTreeMap, path::Path};

use mimecraft::{
    AddressList, ContentDisposition, ContentType, DateTimeOffset, HeaderList, MimeEntity,
    MimeMessage, MimeParser, ParameterList,
    tnef::{TnefConversionLossKind as Loss, TnefMessage, TnefReader},
};
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
struct Node {
    depth: usize,
    kind: String,
    headers: Vec<(String, String)>,
    content: Option<(usize, String)>,
}

impl Node {
    fn new(depth: usize, kind: &str) -> Self {
        Self {
            depth,
            kind: kind.to_owned(),
            headers: Vec::new(),
            content: None,
        }
    }
}

fn unescape(value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        result.push(if ch == '\\' {
            match chars.next().unwrap() {
                '\\' => '\\',
                'r' => '\r',
                'n' => '\n',
                't' => '\t',
                'u' => char::from_u32(
                    u32::from_str_radix(&chars.by_ref().take(4).collect::<String>(), 16).unwrap(),
                )
                .unwrap(),
                other => panic!("unknown snapshot escape: {other}"),
            }
        } else {
            ch
        });
    }
    result
}

fn date(value: &str) -> String {
    let date = if value.len() == 20 && value.ends_with('Z') {
        DateTimeOffset::new(
            value[..4].parse().unwrap(),
            value[5..7].parse().unwrap(),
            value[8..10].parse().unwrap(),
            value[11..13].parse().unwrap(),
            value[14..16].parse().unwrap(),
            value[17..19].parse().unwrap(),
            0,
        )
        .unwrap()
    } else {
        DateTimeOffset::parse(value.as_bytes()).unwrap()
    };
    date.unix_timestamp().to_string()
}

fn parameters(value: &str, parameters: &ParameterList) -> String {
    let mut fields: Vec<_> = parameters
        .iter()
        .map(|parameter| {
            (
                parameter.name().to_ascii_lowercase(),
                parameter.value().to_owned(),
            )
        })
        .collect();
    fields.retain(|(name, _)| name != "boundary");
    for (name, value) in &mut fields {
        if name.ends_with("-date") {
            *value = date(value);
        }
    }
    fields.sort();
    format!("{value}; {fields:?}")
}

fn header(field: &str, value: &str, snapshot: bool) -> Option<(String, String)> {
    let field = field.to_ascii_lowercase();
    let value = match field.as_str() {
        // Rust chooses safe body encodings eagerly. MimeKit often leaves them
        // unspecified until serialization; compare decoded content instead.
        "content-transfer-encoding" => return None,
        "content-type" => {
            let ty = ContentType::parse(value.as_bytes()).unwrap();
            parameters(&ty.mime_type(), ty.parameters())
        }
        "content-disposition" if snapshot => {
            // Upstream descriptions contain decoded, unquoted parameter values.
            let mut pieces = value.split("; ");
            let disposition = pieces.next().unwrap();
            let mut params = ParameterList::new();
            for piece in pieces {
                let (name, value) = piece.split_once('=').unwrap();
                params.set(name, value).unwrap();
            }
            parameters(disposition, &params)
        }
        "content-disposition" => {
            let disposition = ContentDisposition::parse(value.as_bytes()).unwrap();
            parameters(disposition.disposition(), disposition.parameters())
        }
        "date" => date(value),
        "from" | "to" | "cc" | "bcc" | "sender" | "reply-to" => {
            // Address authoring canonicalizes DNS domain case in Rust.
            AddressList::parse(value.as_bytes())
                .unwrap()
                .to_header(&field)
                .unwrap()
                .decode_value()
                .text
        }
        _ => value.to_owned(),
    };
    Some((field, value))
}

fn expected(path: &Path) -> Vec<Node> {
    let snapshot = std::fs::read_to_string(path).unwrap();
    let mut nodes: Vec<Node> = Vec::new();
    for line in snapshot.lines() {
        let text = line.trim_start();
        let depth = (line.len() - text.len()) / 2;
        if text == "message" || text == "(no body)" {
            nodes.push(Node::new(depth, text));
        } else if let Some(content) = text.strip_prefix("content: ") {
            let fields: Vec<_> = content.split_whitespace().collect();
            nodes.last_mut().unwrap().content = Some((
                fields[1].strip_prefix("length=").unwrap().parse().unwrap(),
                fields[2].strip_prefix("sha256=").unwrap().to_owned(),
            ));
        } else if text.starts_with("text: ") {
            // Full decoded hashes cover the truncated text previews as well.
        } else if let Some((field, value)) = text.split_once(": ") {
            if let Some(header) = header(field, &unescape(value), true) {
                nodes.last_mut().unwrap().headers.push(header);
            }
        } else {
            let (class, mime) = text.split_once(' ').unwrap();
            assert!(matches!(
                class,
                "Multipart"
                    | "MultipartAlternative"
                    | "MultipartRelated"
                    | "TextPart"
                    | "MimePart"
                    | "TnefPart"
                    | "MessagePart"
            ));
            nodes.push(Node::new(depth, mime));
        }
    }
    nodes
}

fn ids(message: &MimeMessage, source: &TnefMessage) -> BTreeMap<String, String> {
    let original: Vec<_> = source
        .attachments()
        .iter()
        .filter_map(|attachment| attachment.content_id().unwrap().map(|id| id.into_owned()))
        .collect();
    let mut ids = BTreeMap::new();
    // Generated image IDs are random. Normalize the headers and body references
    // together, without touching identifiers supplied by the original stream.
    for part in message.leaf_parts() {
        if let Some(id) = part.content_id().unwrap()
            && !original
                .iter()
                .any(|original| original.trim().trim_matches(['<', '>']) == id.as_str())
        {
            let replacement = format!("generated-{}@snapshot", ids.len());
            assert!(ids.insert(id.as_str().to_owned(), replacement).is_none());
        }
    }
    ids
}

fn replace_ids(value: &str, ids: &BTreeMap<String, String>) -> String {
    let mut value = value.to_owned();
    for (old, new) in ids {
        value = value.replace(old, new);
    }
    value
}

fn headers(fields: &HeaderList, ids: &BTreeMap<String, String>) -> Vec<(String, String)> {
    fields
        .iter()
        .filter_map(|field| {
            let decoded = field.decode_value();
            assert!(decoded.issues.is_empty(), "{decoded:?}");
            header(
                field.field().unwrap(),
                &replace_ids(&decoded.text, ids),
                false,
            )
        })
        .collect()
}

fn describe(message: &mut MimeMessage, ids: &BTreeMap<String, String>, fixture: &str) -> Vec<Node> {
    fn describe_entity(
        entity: &mut MimeEntity,
        depth: usize,
        ids: &BTreeMap<String, String>,
        fixture: &str,
        nodes: &mut Vec<Node>,
    ) {
        let mut node = Node::new(depth, "");
        node.headers = headers(entity.headers(), ids);
        match entity {
            MimeEntity::Multipart(part) => {
                node.kind = part.content_type().unwrap().unwrap().mime_type();
                nodes.push(node);
                for child in part.children_mut() {
                    describe_entity(child, depth + 1, ids, fixture, nodes);
                }
            }
            MimeEntity::Message(_) => panic!("default corpus conversion retains embedded TNEF"),
            MimeEntity::Part(part) => {
                node.kind = part
                    .content_type()
                    .unwrap()
                    .map(|ty| ty.mime_type())
                    .unwrap_or_else(|| "text/plain".into());
                let mut bytes = Vec::new();
                part.content_mut().decode_to(&mut bytes).unwrap();
                if node.kind.starts_with("text/") {
                    // Match the upstream Unix2DosFilter without transcoding
                    // attachments or altering their original charset bytes.
                    let mut canonical = Vec::new();
                    for (index, &byte) in bytes.iter().enumerate() {
                        if byte == b'\n' && (index == 0 || bytes[index - 1] != b'\r') {
                            canonical.push(b'\r');
                        }
                        canonical.push(byte);
                    }
                    bytes = canonical;
                    if !ids.is_empty() && !part.headers().contains("Content-Disposition") {
                        bytes = replace_ids(std::str::from_utf8(&bytes).unwrap(), ids).into_bytes();
                    }
                    if node.kind == "text/calendar" {
                        // Only the library's product identifier differs.
                        bytes = std::str::from_utf8(&bytes)
                            .unwrap()
                            .replace(
                                "PRODID:-//mimecraft//EN\r\n",
                                "PRODID:-//.NET Foundation//MimeKit//EN\r\n",
                            )
                            .into_bytes();
                    }
                    if fixture == "multi-value-attribute" && node.kind == "text/html" {
                        // Encapsulated HTML preserves <br> in Rust; MimeKit's
                        // extractor writes the equivalent XHTML spelling.
                        bytes = std::str::from_utf8(&bytes)
                            .unwrap()
                            .replace("<br>", "<br/>")
                            .into_bytes();
                    }
                }
                node.content = Some((bytes.len(), format!("{:x}", Sha256::digest(&bytes))));
                nodes.push(node);
            }
        }
    }
    let mut root = Node::new(0, "message");
    root.headers = headers(message.headers(), ids);
    let mut nodes = vec![root];
    if let Some(body) = message.body_mut() {
        describe_entity(body, 1, ids, fixture, &mut nodes);
    } else {
        nodes.push(Node::new(1, "(no body)"));
    }
    nodes
}

#[test]
fn upstream_corpus_conversion_snapshots_and_mime_round_trips() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("tnef");
    let mut paths: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "tnef")
        })
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 19);
    for path in paths {
        let fixture = path.file_stem().unwrap().to_str().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let source = TnefReader::new(bytes.as_slice()).load_message().unwrap();
        let mut result = source.to_mime_message().unwrap();
        let losses: Vec<_> = result
            .losses
            .iter()
            .map(|loss| (loss.kind, loss.attachment_index))
            .collect();
        let expected_losses = match fixture {
            "MAPI_OBJECT" => vec![(Loss::OleObjectNotRendered, Some(0))],
            "winmail" => vec![
                (Loss::OleObjectNotRendered, Some(0)),
                (Loss::OleObjectNotRendered, Some(1)),
            ],
            "body" => vec![(Loss::UnparsableRecipient, None)],
            _ => Vec::new(),
        };
        assert_eq!(losses, expected_losses, "{fixture}");
        let ids = ids(&result.message, &source);
        let actual = describe(&mut result.message, &ids, fixture);
        assert_eq!(
            actual,
            expected(&path.with_extension("snapshot")),
            "{fixture}"
        );
        let mut wire = Vec::new();
        result.message.write_to(&mut wire).unwrap();
        let mut parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
        // A headers-only message parses as an implicit empty text/plain part.
        if result.message.body().is_none() {
            let MimeEntity::Part(part) = parsed.body_mut().unwrap() else {
                panic!("implicit text part")
            };
            let mut content = Vec::new();
            part.content_mut().decode_to(&mut content).unwrap();
            assert!(part.headers().is_empty() && content.is_empty());
            parsed.set_body(None);
        }
        assert_eq!(
            describe(&mut parsed, &ids, fixture),
            actual,
            "{fixture}: MIME round trip"
        );
    }
}

#[test]
fn upstream_descriptions_are_deterministic() {
    // Use an image-bearing fixture so random CIDs as well as boundaries change.
    let bytes = include_bytes!("data/tnef/attachments.tnef");
    let source = TnefReader::new(bytes.as_slice()).load_message().unwrap();
    let mut first = source.to_mime_message().unwrap().message;
    let mut second = source.to_mime_message().unwrap().message;
    let first_ids = ids(&first, &source);
    let second_ids = ids(&second, &source);
    assert!(!first_ids.is_empty());
    assert_eq!(
        describe(&mut first, &first_ids, "attachments"),
        describe(&mut second, &second_ids, "attachments")
    );
}

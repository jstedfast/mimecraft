// Adapted from MimeKit's UnitTests/MimeIteratorTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom};

use mimecraft::{
    Header, HeaderList, MessagePart, MimeContent, MimeEntity, MimeIterator,
    MimeIteratorPositionError, MimeMessage, MimePart, MimePathParseError, Multipart,
};

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("tree navigation must not read sources")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("tree navigation must not seek sources")
    }
}

fn leaf() -> MimeEntity<NoIo> {
    MimePart::from_parts(HeaderList::new(), MimeContent::new(NoIo)).into()
}

fn multipart(children: Vec<MimeEntity<NoIo>>) -> MimeEntity<NoIo> {
    Multipart::from_parts(HeaderList::new(), children, Vec::new(), Vec::new()).into()
}

fn attached(body: MimeEntity<NoIo>) -> MimeEntity<NoIo> {
    MessagePart::new(MimeMessage::with_body(body)).into()
}

fn example() -> MimeMessage<NoIo> {
    // The same tree as MimeKit's IMAP example, including multipart bodies
    // nested inside attached messages and an ordinary nested multipart.
    MimeMessage::with_body(multipart(vec![
        leaf(),
        leaf(),
        attached(multipart(vec![leaf(), leaf()])),
        multipart(vec![
            leaf(),
            attached(multipart(vec![leaf(), multipart(vec![leaf(), leaf()])])),
        ]),
    ]))
}

const PATHS: [&str; 15] = [
    "0", "1", "2", "3", "3.0", "3.1", "3.2", "4", "4.1", "4.2", "4.2.0", "4.2.1", "4.2.2",
    "4.2.2.1", "4.2.2.2",
];
const DEPTHS: [usize; 15] = [0, 1, 1, 1, 2, 3, 3, 1, 2, 2, 3, 4, 4, 5, 5];
const PARENTS: [Option<usize>; 15] = [
    None,
    Some(0),
    Some(0),
    Some(0),
    Some(3),
    Some(4),
    Some(4),
    Some(0),
    Some(7),
    Some(7),
    Some(9),
    Some(10),
    Some(10),
    Some(12),
    Some(12),
];
const INDEXES: [Option<usize>; 15] = [
    None,
    Some(0),
    Some(1),
    Some(2),
    None,
    Some(0),
    Some(1),
    Some(3),
    Some(0),
    Some(1),
    None,
    Some(0),
    Some(1),
    Some(0),
    Some(1),
];

fn paths<R>(iter: &mut MimeIterator<'_, R>) -> Vec<String> {
    let mut result = Vec::new();
    while iter.next().is_some() {
        result.push(iter.path_specifier().unwrap());
    }
    result
}

fn assert_unpositioned<R>(iter: &mut MimeIterator<'_, R>) {
    assert!(iter.current().is_none());
    assert!(iter.parent().is_none());
    assert!(iter.depth().is_none());
    assert!(iter.index().is_none());
    assert!(iter.current_message().is_none());
    assert!(iter.path_specifier().is_none());
    assert!(iter.ancestor(0).is_none());
    assert!(iter.ancestor(1).is_none());
    assert_eq!(iter.skip_children(), Err(MimeIteratorPositionError));
}

#[test]
fn traversal_reports_paths_depth_parents_indexes_and_containing_messages() {
    let message = example();
    let mut iter = message.iter();
    assert!(std::ptr::eq(iter.message(), &message));
    assert_unpositioned(&mut iter);
    for _ in 0..2 {
        let mut visited: Vec<&MimeEntity<NoIo>> = Vec::new();
        while let Some(entity) = iter.next() {
            let i = visited.len();
            assert!(std::ptr::eq(entity, iter.current().unwrap()));
            assert_eq!(iter.path_specifier().as_deref(), Some(PATHS[i]));
            assert_eq!(iter.depth(), Some(DEPTHS[i]));
            assert_eq!(iter.index(), INDEXES[i]);
            if let Some(parent) = PARENTS[i] {
                assert!(std::ptr::eq(iter.parent().unwrap(), visited[parent]));
            } else {
                assert!(iter.parent().is_none());
            }
            if let Some(index) = iter.index() {
                let MimeEntity::Multipart(parent) = iter.parent().unwrap() else {
                    panic!("indexed entities must have multipart parents");
                };
                assert!(std::ptr::eq(entity, &parent.children()[index]));
            }
            let wrapper = if (4..=6).contains(&i) {
                Some(3)
            } else if i >= 10 {
                Some(9)
            } else {
                None
            };
            let containing = if let Some(wrapper) = wrapper {
                let MimeEntity::Message(part) = visited[wrapper] else {
                    panic!("expected an attached message");
                };
                part.message().unwrap()
            } else {
                &message
            };
            assert!(std::ptr::eq(iter.current_message().unwrap(), containing));
            assert!(std::ptr::eq(
                message.entity_at(PATHS[i]).unwrap().unwrap(),
                entity
            ));
            visited.push(entity);
        }
        assert_eq!(visited.len(), PATHS.len());
        assert_unpositioned(&mut iter);
        assert!(iter.next().is_none());
        iter.reset();
        assert_unpositioned(&mut iter);
    }
}

#[test]
fn navigation_restores_every_frame_and_next_resumes_after_the_target() {
    let message = example();
    let entities = message.iter().collect::<Vec<_>>();
    let mut iter = message.iter();
    for i in (0..PATHS.len()).rev() {
        assert!(iter.move_to(PATHS[i]).unwrap());
        assert!(std::ptr::eq(iter.current().unwrap(), entities[i]));
        assert_eq!(iter.path_specifier().as_deref(), Some(PATHS[i]));
        assert_eq!(iter.depth(), Some(DEPTHS[i]));
        assert_eq!(iter.index(), INDEXES[i]);
        let mut parent = PARENTS[i];
        for generation in 1..=DEPTHS[i] {
            let index = parent.unwrap();
            assert!(std::ptr::eq(
                iter.ancestor(generation).unwrap(),
                entities[index]
            ));
            parent = PARENTS[index];
        }
        assert!(iter.ancestor(0).is_none());
        assert!(iter.ancestor(DEPTHS[i] + 1).is_none());
        assert!(iter.ancestor(usize::MAX).is_none());
        assert_eq!(paths(&mut iter), PATHS[i + 1..]);
    }
}

#[test]
fn skipping_subtrees_and_leaves_matches_upstream() {
    let message = example();
    let mut iter = message.iter();
    for (path, next) in [
        ("3", Some("4")),
        ("1", Some("2")),
        ("4", None),
        ("4.2.2", None),
        ("3.0", Some("4")),
        ("0", None),
    ] {
        assert!(iter.move_to(path).unwrap());
        iter.skip_children().unwrap();
        iter.skip_children().unwrap();
        assert_eq!(iter.next().is_some(), next.is_some());
        assert_eq!(iter.path_specifier().as_deref(), next);
    }
    iter.reset();
    let mut actual = Vec::new();
    while let Some(entity) = iter.next() {
        actual.push(iter.path_specifier().unwrap());
        if matches!(entity, MimeEntity::Message(_)) {
            iter.skip_children().unwrap();
        }
    }
    assert_eq!(actual, ["0", "1", "2", "3", "4", "4.1", "4.2"]);
}

#[test]
fn reset_and_navigation_cancel_skips_but_syntax_errors_preserve_them() {
    let message = example();
    let mut iter = message.iter();
    iter.next().unwrap();
    iter.skip_children().unwrap();
    iter.reset();
    iter.next().unwrap();
    iter.next().unwrap();
    assert_eq!(iter.path_specifier().as_deref(), Some("1"));

    assert!(iter.move_to("4").unwrap());
    iter.skip_children().unwrap();
    assert!(iter.move_to("4").unwrap());
    iter.next().unwrap();
    assert_eq!(iter.path_specifier().as_deref(), Some("4.1"));

    assert!(iter.move_to("3").unwrap());
    iter.skip_children().unwrap();
    let before = iter.current().unwrap();
    assert!(iter.move_to("999.invalid").is_err());
    assert!(std::ptr::eq(iter.current().unwrap(), before));
    iter.next().unwrap();
    assert_eq!(iter.path_specifier().as_deref(), Some("4"));
}

#[test]
fn valid_missing_paths_exhaust_the_iterator_without_partial_positions() {
    let message = example();
    let mut iter = message.iter();
    for path in [
        "0.1",
        "4.0",
        "3.0.1",
        "5",
        "3.3",
        "4.2.2.3",
        "1.1",
        "4.2.2.1.1",
        "2147483647",
    ] {
        assert!(iter.move_to("3").unwrap());
        iter.skip_children().unwrap();
        assert!(!iter.move_to(path).unwrap(), "{path}");
        assert_unpositioned(&mut iter);
        assert!(iter.next().is_none());
        assert!(message.entity_at(path).unwrap().is_none());
    }
}

#[test]
fn malformed_paths_report_exact_offsets_and_do_not_change_position() {
    use MimePathParseError::{ComponentOverflow, EmptyComponent, InvalidCharacter};
    let message = example();
    let mut iter = message.iter();
    assert!(iter.move_to("4.2").unwrap());
    let before = iter.current().unwrap();
    for (path, expected) in [
        ("", EmptyComponent { offset: 0 }),
        (".", EmptyComponent { offset: 0 }),
        ("1.", EmptyComponent { offset: 2 }),
        ("1..2", EmptyComponent { offset: 2 }),
        ("xyz", InvalidCharacter { offset: 0 }),
        ("-1", InvalidCharacter { offset: 0 }),
        ("+1", InvalidCharacter { offset: 0 }),
        (" 1", InvalidCharacter { offset: 0 }),
        ("1 ", InvalidCharacter { offset: 1 }),
        ("1.\t2", InvalidCharacter { offset: 2 }),
        ("1.\u{ff11}", InvalidCharacter { offset: 2 }),
        ("1\0", InvalidCharacter { offset: 1 }),
        ("1\r\n", InvalidCharacter { offset: 1 }),
        ("2147483648", ComponentOverflow { offset: 9 }),
        ("1.2147483648", ComponentOverflow { offset: 11 }),
        ("99999999999999999999999", ComponentOverflow { offset: 9 }),
    ] {
        assert_eq!(iter.move_to(path), Err(expected), "{path}");
        assert_eq!(message.entity_at(path).unwrap_err(), expected);
        assert!(std::ptr::eq(iter.current().unwrap(), before));
        assert_eq!(iter.path_specifier().as_deref(), Some("4.2"));
        assert!(!expected.to_string().is_empty());
    }
}

#[test]
fn invalid_navigation_preserves_before_first_and_exhausted_states() {
    let message = example();
    let mut iter = message.iter();
    assert!(iter.move_to("").is_err());
    assert_unpositioned(&mut iter);
    assert!(iter.next().is_some());
    assert_eq!(iter.path_specifier().as_deref(), Some("0"));
    assert_eq!(iter.by_ref().count(), PATHS.len() - 1);
    assert!(iter.move_to("1.").is_err());
    assert_unpositioned(&mut iter);
    assert!(iter.next().is_none());
    assert!(iter.move_to("1").unwrap());
    assert_eq!(iter.path_specifier().as_deref(), Some("1"));
}

#[test]
fn leading_zero_components_are_accepted_and_paths_are_canonicalized() {
    let message = example();
    let mut iter = message.iter();
    for (input, expected) in [
        ("000", "0"),
        ("03.00", "3.0"),
        ("004.002.0002.01", "4.2.2.1"),
    ] {
        assert!(iter.move_to(input).unwrap());
        assert_eq!(iter.path_specifier().as_deref(), Some(expected));
    }
    assert!(iter.move_to(&format!("{}1", "0".repeat(4096))).unwrap());
    assert_eq!(iter.path_specifier().as_deref(), Some("1"));
}

#[test]
fn empty_messages_and_containers_require_no_source_traits() {
    let message: MimeMessage<()> = MimeMessage::new();
    let mut iter = message.iter();
    assert!(iter.next().is_none());
    assert_unpositioned(&mut iter);
    assert!(!iter.move_to("1").unwrap());
    assert!(message.entity_at("invalid").is_err());

    let multipart =
        Multipart::<()>::from_parts(HeaderList::new(), Vec::new(), Vec::new(), Vec::new());
    let message = MimeMessage::with_body(multipart.into());
    assert_eq!(paths(&mut message.iter()), ["0"]);
    assert!(message.entity_at("0").unwrap().is_some());
    assert!(message.entity_at("1").unwrap().is_none());
}

#[test]
fn nonmultipart_and_nested_message_bodies_have_explicit_part_numbers() {
    let message = MimeMessage::with_body(leaf());
    assert_eq!(paths(&mut message.iter()), ["1"]);
    assert!(std::ptr::eq(
        message.entity_at("1").unwrap().unwrap(),
        message.body().unwrap()
    ));
    for path in ["0", "2", "1.0", "1.1"] {
        assert!(message.entity_at(path).unwrap().is_none());
    }

    let message = MimeMessage::with_body(attached(attached(leaf())));
    let mut iter = message.iter();
    assert_eq!(paths(&mut iter), ["1", "1.1", "1.1.1"]);
    assert!(iter.move_to("1.1.1").unwrap());
    assert_eq!(iter.depth(), Some(2));
    assert_eq!(iter.index(), None);
    let MimeEntity::Message(part) = iter.parent().unwrap() else {
        panic!("expected a message wrapper");
    };
    assert!(std::ptr::eq(
        iter.current_message().unwrap(),
        part.message().unwrap()
    ));
    for path in ["0", "1.0", "1.2", "1.1.0", "1.1.2"] {
        assert!(message.entity_at(path).unwrap().is_none());
    }
}

#[test]
fn absent_attached_messages_and_bodies_are_terminal_nodes() {
    let message = MimeMessage::with_body(multipart(vec![
        MessagePart::from_parts(HeaderList::new(), None).into(),
        MessagePart::new(MimeMessage::new()).into(),
        multipart(Vec::new()),
        leaf(),
    ]));
    assert_eq!(paths(&mut message.iter()), ["0", "1", "2", "3", "4"]);
    for path in ["1.0", "1.1", "2.0", "2.1", "3.0", "3.1"] {
        assert!(message.entity_at(path).unwrap().is_none());
    }
}

#[test]
fn navigation_uses_variants_not_headers_and_preserves_raw_metadata() {
    let mut message = example();
    message
        .headers_mut()
        .add(Header::parse(b"Subject: unchanged\n").unwrap());
    message
        .body_mut()
        .unwrap()
        .headers_mut()
        .add(Header::parse(b"Content-Type: invalid\n").unwrap());
    let before = message.body().unwrap().headers().clone();
    assert_eq!(paths(&mut message.iter()), PATHS);
    assert!(message.entity_at("4.2.2.2").unwrap().is_some());
    assert_eq!(message.body().unwrap().headers(), &before);
    assert_eq!(
        message.headers().get("Subject").unwrap().raw(),
        b"Subject: unchanged\n"
    );
}

#[test]
fn existing_body_and_leaf_traversals_keep_their_distinct_scope() {
    let message = example();
    let body_paths = ["1", "2", "3", "4.1", "4.2"];
    let bodies = message.body_parts().collect::<Vec<_>>();
    assert_eq!(bodies.len(), body_paths.len());
    for (entity, path) in bodies.into_iter().zip(body_paths) {
        assert!(std::ptr::eq(
            entity,
            message.entity_at(path).unwrap().unwrap()
        ));
    }
    let all_leaves = message
        .iter()
        .filter(|entity| matches!(entity, MimeEntity::Part(_)))
        .count();
    assert_eq!(message.leaf_parts().count(), all_leaves);
    assert_eq!(all_leaves, 8);
}

#[test]
fn ten_thousand_levels_traverse_and_navigate_without_recursion() {
    const DEPTH: usize = 10_000;
    let mut root = leaf();
    for _ in 0..DEPTH {
        root = multipart(vec![root]);
    }
    let mut message = MimeMessage::with_body(root);
    let mut iter = message.iter();
    assert_eq!(iter.by_ref().count(), DEPTH + 1);
    let path = vec!["1"; DEPTH].join(".");
    assert!(!iter.move_to(&format!("0.{path}")).unwrap());
    assert!(iter.move_to(&path).unwrap());
    assert_eq!(iter.depth(), Some(DEPTH));
    assert!(matches!(iter.current().unwrap(), MimeEntity::Part(_)));
    assert_eq!(iter.path_specifier().as_deref(), Some(path.as_str()));
    assert!(iter.next().is_none());

    // Model drop is recursive; dismantle this stress-test tree iteratively.
    let mut entity = message.set_body(None).unwrap();
    while let MimeEntity::Multipart(part) = entity {
        let (_, mut children, _, _) = part.into_parts();
        entity = children.pop().unwrap();
    }
}

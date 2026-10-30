// Adapted from MimeKit's Benchmarks/MimeMessageBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentEncoding, EncodingConstraint, MimeContent, MimeEntity, MimeMessage, MimePart, Multipart,
    TextPart,
};
use std::{cell::RefCell, hint::black_box, io::Cursor};

use crate::fixtures;

pub const CASES: [(&str, EncodingConstraint); 3] = [
    ("None", EncodingConstraint::None),
    ("SevenBit", EncodingConstraint::SevenBit),
    ("EightBit", EncodingConstraint::EightBit),
];

pub struct Workload {
    pub constraint: EncodingConstraint,
    pub input_bytes: usize,
    message: RefCell<MimeMessage>,
}

pub fn workload(constraint: EncodingConstraint) -> Workload {
    let lorem = fixtures::attachment("lorem-ipsum.txt");
    let girl = fixtures::attachment("girl.jpg");
    let input_bytes = lorem.len() + girl.len();
    let text = std::str::from_utf8(&lorem)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let mut multipart = Multipart::with_boundary("mixed", "benchmark").unwrap();
    multipart
        .children_mut()
        .push(TextPart::new("plain", text).unwrap().into_part().into());
    multipart
        .children_mut()
        .push(MimePart::new(MimeContent::new(Cursor::new(girl))).into());
    Workload {
        constraint,
        input_bytes,
        message: RefCell::new(MimeMessage::with_body(multipart.into())),
    }
}

impl Workload {
    pub fn run(&self) {
        let mut message = self.message.borrow_mut();
        let MimeEntity::Multipart(body) = message.body_mut().unwrap() else {
            unreachable!()
        };
        // Like upstream, reuse the message but reset every leaf's target
        // encoding before each Prepare, so an earlier result cannot skip work.
        for child in body.children_mut() {
            let MimeEntity::Part(part) = child else {
                unreachable!()
            };
            part.set_content_transfer_encoding(ContentEncoding::Unspecified);
        }
        message.prepare(black_box(self.constraint)).unwrap();
        black_box(&*message);
    }

    pub fn check(&self) {
        // Run twice to exercise both the first preparation and reuse.
        for _ in 0..2 {
            self.run();
            let message = self.message.borrow();
            let encodings = message
                .leaf_parts()
                .map(|p| p.content_transfer_encoding().unwrap())
                .collect::<Vec<_>>();
            let binary = if self.constraint == EncodingConstraint::None {
                ContentEncoding::Binary
            } else {
                ContentEncoding::Base64
            };
            assert_eq!(encodings, [ContentEncoding::Unspecified, binary]);
        }
    }
}

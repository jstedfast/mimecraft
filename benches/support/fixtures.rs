// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::path::Path;

use crate::fingerprint::fingerprint;

pub fn attachment(name: &str) -> Vec<u8> {
    let expected = match name {
        "girl.jpg" => (130292, 0xc21813b85e0e0385),
        "lorem-ipsum.txt" => (2568, 0x0f79f7ecad15bc49),
        _ => panic!("unknown attachment fixture: {name}"),
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("attachments")
        .join(name);
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(fingerprint(&bytes), expected, "{name}");
    bytes
}

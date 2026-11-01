// Original workloads: MimeKit has no TNEF message-body benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{hint::black_box, io::Read};

use mimecraft::{
    charsets::Charset,
    tnef::{TnefMessageBody as Body, TnefPropertyTag as Tag},
};

use super::rtf_compressed_workloads;

pub const CASES: [&str; 5] = [
    "String8",
    "Unicode",
    "Html",
    "CompressedRtf",
    "UncompressedRtf",
];
pub const MODES: [Mode; 3] = [Mode::Construct, Mode::Stream, Mode::Text];

#[derive(Clone, Copy)]
pub enum Mode {
    Construct,
    Stream,
    Text,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Construct => "Construct",
            Self::Stream => "Stream",
            Self::Text => "Text",
        }
    }
}

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    body: Body,
    tag: Tag,
    charset: Charset,
    raw: Vec<u8>,
    decoded: Vec<u8>,
    text: String,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub bytes: usize,
    pub checksum: u64,
}

fn accumulate(summary: &mut Summary, bytes: &[u8]) {
    summary.bytes += bytes.len();
    summary.checksum = bytes.iter().fold(summary.checksum, |sum, &byte| {
        sum.wrapping_add(u64::from(byte))
    });
}

pub fn workload(name: &'static str) -> Workload {
    let utf8 = Charset::UTF8;
    let (tag, charset, raw, decoded, text) = match name {
        "String8" => {
            let bytes = b"Caf\xe9\n".repeat(4096);
            (
                Tag::BODY_A,
                Charset::from_label("windows-1252").unwrap(),
                bytes.clone(),
                bytes,
                "Caf\u{e9}\n".repeat(4096),
            )
        }
        "Unicode" => {
            let text = "MAPI body caf\u{e9}\n".repeat(4096);
            let bytes: Vec<_> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            (Tag::BODY_W, utf8, bytes.clone(), bytes, text)
        }
        "Html" => {
            let text = "<p>caf\u{e9}</p>\n".repeat(4096);
            (
                Tag::BODY_HTML_B,
                utf8,
                text.as_bytes().to_vec(),
                text.as_bytes().to_vec(),
                text,
            )
        }
        "CompressedRtf" | "UncompressedRtf" => {
            let case = rtf_compressed_workloads::workload(if name == "CompressedRtf" {
                "References"
            } else {
                "Uncompressed"
            });
            let text = String::from_utf8(case.expected.clone()).unwrap();
            (Tag::RTF_COMPRESSED, utf8, case.input, case.expected, text)
        }
        _ => panic!("unknown TNEF body workload"),
    };
    let mut input = raw.clone();
    if tag == Tag::BODY_A {
        input.extend_from_slice(&[0, 0]);
    }
    if tag == Tag::BODY_W {
        input.extend_from_slice(&[0, 0, 0, 0]);
    }
    let body = Body::new(tag, input.clone(), charset).unwrap();
    Workload {
        name,
        input,
        body,
        tag,
        charset,
        raw,
        decoded,
        text,
    }
}

pub fn read(case: &Workload, mode: Mode) -> Summary {
    let case = black_box(case);
    let mut summary = Summary::default();
    match mode {
        Mode::Construct => {
            let body = Body::new(case.tag, case.input.clone(), case.charset).unwrap();
            summary.bytes = black_box(&body).len();
        }
        Mode::Stream => {
            let mut reader = case.body.open_decoded_read();
            let mut buffer = [0; 4096];
            loop {
                let count = reader.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                accumulate(&mut summary, black_box(&buffer[..count]));
            }
        }
        Mode::Text => {
            let text = case.body.text().unwrap();
            accumulate(&mut summary, black_box(text.as_bytes()));
        }
    }
    summary
}

impl Workload {
    pub fn check(&self) {
        assert_eq!(self.body.as_bytes(), self.raw);
        let mut decoded = Vec::new();
        self.body
            .open_decoded_read()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, self.decoded);
        assert_eq!(self.body.text().unwrap(), self.text);
        for mode in MODES {
            let mut expected = Summary::default();
            match mode {
                Mode::Construct => expected.bytes = self.raw.len(),
                Mode::Stream => accumulate(&mut expected, &self.decoded),
                Mode::Text => accumulate(&mut expected, self.text.as_bytes()),
            }
            assert_eq!(read(self, mode), expected, "{}/{}", self.name, mode.name());
        }
    }
}

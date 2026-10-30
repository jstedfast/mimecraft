// Adapted from MimeKit's Benchmarks/AddressParserBenchmarks.cs and
// Benchmarks/Utils/Rfc2047Benchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::hint::black_box;

#[path = "support/fingerprint.rs"]
mod fingerprint;
#[path = "support/timing.rs"]
mod timing;
#[path = "support/headers.rs"]
mod workloads;

fn main() {
    timing::heading();
    println!(
        "Fresh address parses/RFC 2047 decoding, including result allocation and destruction."
    );
    for name in workloads::CASES {
        let case = workloads::workload(name);
        case.check();
        timing::measure(
            &format!("headers/AddressParser_ParseBytes/{name}"),
            case.text.len(),
            || {
                black_box(workloads::parse_bytes(&case));
            },
        );
        timing::measure(
            &format!("headers/AddressParser_ParseString/{name}"),
            case.text.len(),
            || {
                black_box(workloads::parse_string(&case));
            },
        );
    }
    workloads::check_rfc2047();
    timing::measure(
        "headers/Rfc2047_DecodeText/ILoveMimeSoMuch",
        workloads::RFC2047_INPUT.len(),
        || {
            drop(black_box(workloads::decode_text()));
        },
    );
    timing::measure(
        "headers/Rfc2047_DecodePhrase/ILoveMimeSoMuch",
        workloads::RFC2047_INPUT.len(),
        || {
            drop(black_box(workloads::decode_phrase()));
        },
    );
}

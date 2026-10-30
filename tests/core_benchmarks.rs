// Adapted from MimeKit's Benchmarks workloads.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[path = "../benches/support/fingerprint.rs"]
mod fingerprint;
#[path = "../benches/support/fixtures.rs"]
mod fixtures;
#[path = "../benches/support/headers.rs"]
mod headers;

#[path = "../benches/support/filters.rs"]
mod filters;

#[path = "../benches/support/message.rs"]
mod message;

#[test]
fn address_workloads_match_upstream() {
    for name in headers::CASES {
        headers::workload(name).check();
    }
}

#[test]
fn rfc2047_workloads() {
    headers::check_rfc2047();
}

#[test]
fn filter_workloads_match_upstream() {
    for name in filters::CASES {
        let case = filters::workload(name);
        case.check();
        case.run();
        assert_eq!(
            case.bytes_per_operation(),
            case.input.len()
                * if name.starts_with("BestEncoding_") {
                    100
                } else {
                    1
                }
        );
    }
}

#[test]
fn message_preparation_matches_upstream() {
    for (_, constraint) in message::CASES {
        let case = message::workload(constraint);
        assert_eq!(case.input_bytes, 132860);
        case.check();
    }
}

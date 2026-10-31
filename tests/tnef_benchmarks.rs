// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[path = "../benches/support/tnef.rs"]
mod workloads;

#[path = "../benches/support/tnef_properties.rs"]
mod property_workloads;

#[path = "../benches/support/tnef_dates.rs"]
mod date_workloads;

#[path = "../benches/support/tnef_embedded.rs"]
mod embedded_workloads;

#[path = "../benches/support/tnef_conversions.rs"]
mod conversion_workloads;

#[path = "../benches/support/tnef_recipients.rs"]
mod recipient_workloads;

#[path = "../benches/support/rtf_compressed.rs"]
mod rtf_compressed_workloads;

#[test]
fn compressed_rtf_workloads_match_reference_encoder() {
    for name in rtf_compressed_workloads::CASES {
        let case = rtf_compressed_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
        assert!(!case.input.is_empty());
    }
}

#[test]
fn recipient_tnef_workloads_match_generated_fields() {
    for name in recipient_workloads::CASES {
        let case = recipient_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
        assert!(!case.input.is_empty());
    }
    for mode in recipient_workloads::MODES {
        assert!(!mode.name().is_empty());
    }
}

#[test]
fn owned_tnef_access_workloads_match_generated_values() {
    for name in conversion_workloads::CASES {
        let case = conversion_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
        assert!(case.input_length > 0);
    }
}

#[test]
fn embedded_tnef_workloads_match_generated_messages_and_bodies() {
    for name in embedded_workloads::CASES {
        let case = embedded_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
    }
}

#[test]
fn date_tnef_workloads_match_generated_ticks_and_kinds() {
    for name in date_workloads::CASES {
        let case = date_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
    }
}

#[test]
fn raw_tnef_workloads_match_independent_wire_checks() {
    for name in workloads::CASES {
        let case = workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
    }

    for mode in workloads::MODES {
        assert!(!mode.name().is_empty());
    }
}

#[test]
fn property_tnef_workloads_match_generated_values() {
    for name in property_workloads::CASES {
        let case = property_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
    }
    for mode in property_workloads::MODES {
        assert!(!mode.name().is_empty());
    }
}

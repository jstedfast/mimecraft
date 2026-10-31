// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[path = "../benches/support/tnef.rs"]
mod workloads;

#[path = "../benches/support/tnef_properties.rs"]
mod property_workloads;

#[path = "../benches/support/tnef_dates.rs"]
mod date_workloads;

#[path = "../benches/support/tnef_embedded.rs"]
mod embedded_workloads;

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

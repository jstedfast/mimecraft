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

#[path = "../benches/support/rtf_compressed_writer.rs"]
mod rtf_writer_workloads;

#[path = "../benches/support/tnef_writer.rs"]
mod writer_workloads;

#[path = "../benches/support/tnef_calendar.rs"]
mod calendar_workloads;

#[path = "../benches/support/tnef_time_zone.rs"]
mod time_zone_workloads;

#[path = "../benches/support/tnef_recurrence.rs"]
mod recurrence_workloads;

#[test]
fn recurrence_workloads_match_independent_fields_text_and_diagnostics() {
    for name in recurrence_workloads::CASES {
        let case = recurrence_workloads::workload(name);
        assert_eq!(case.name, name);
        assert!(case.bytes > 0);
        case.check();
    }
}

#[test]
fn time_zone_workloads_match_known_dates_rules_and_calendar_output() {
    for name in time_zone_workloads::CASES {
        let case = time_zone_workloads::workload(name);
        assert_eq!(case.name, name);
        assert!(case.operations > 0);
        case.check();
    }
}

#[test]
fn calendar_writer_workloads_match_independent_utf8_content_lines() {
    for name in calendar_workloads::CASES {
        let case = calendar_workloads::workload(name);
        assert_eq!(case.name, name);
        assert!(case.output_length > 0);
        case.check();
    }
}

#[test]
fn tnef_writer_workloads_match_independent_framing_and_full_content() {
    for name in writer_workloads::CASES {
        let case = writer_workloads::workload(name);
        assert_eq!(case.name, name);
        assert!(case.output_length > 40);
        case.check();
    }
}

#[test]
fn compressed_rtf_writer_workloads_match_headers_crc_and_full_content() {
    for name in rtf_writer_workloads::CASES {
        let case = rtf_writer_workloads::workload(name);
        assert_eq!(case.name, name);
        assert!(!case.input.is_empty());
        case.check();
    }
}

#[path = "../benches/support/tnef_bodies.rs"]
mod body_workloads;

#[path = "../benches/support/tnef_attachments.rs"]
mod attachment_workloads;

#[path = "../benches/support/tnef_messages.rs"]
mod message_workloads;

#[test]
fn whole_message_workloads_match_independent_content_and_metadata() {
    for name in message_workloads::CASES {
        let case = message_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
        assert!(!case.input.is_empty());
    }
    for mode in message_workloads::MODES {
        assert!(!mode.name().is_empty());
    }
}

#[test]
fn attachment_workloads_match_metadata_and_content() {
    for name in attachment_workloads::CASES {
        let case = attachment_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
        assert!(!case.input.is_empty());
    }
    for mode in attachment_workloads::MODES {
        assert!(!mode.name().is_empty());
    }
}

#[test]
fn body_workloads_match_raw_decoded_and_text_references() {
    for name in body_workloads::CASES {
        let case = body_workloads::workload(name);
        case.check();
        assert!(!case.name.is_empty());
        assert!(!case.input.is_empty());
    }
    for mode in body_workloads::MODES {
        assert!(!mode.name().is_empty());
    }
}

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

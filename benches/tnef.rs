// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[path = "support/tnef.rs"]
mod workloads;

#[path = "support/tnef_properties.rs"]
mod property_workloads;

#[path = "support/tnef_dates.rs"]
mod date_workloads;

#[path = "support/tnef_embedded.rs"]
mod embedded_workloads;

#[path = "support/tnef_conversions.rs"]
mod conversion_workloads;

#[path = "support/tnef_recipients.rs"]
mod recipient_workloads;

#[path = "support/rtf_compressed.rs"]
mod rtf_compressed_workloads;

#[path = "support/rtf_compressed_writer.rs"]
mod rtf_writer_workloads;

#[path = "support/tnef_writer.rs"]
mod writer_workloads;

#[path = "support/tnef_calendar.rs"]
mod calendar_workloads;

#[path = "support/tnef_time_zone.rs"]
mod time_zone_workloads;

#[path = "support/tnef_recurrence.rs"]
mod recurrence_workloads;

#[path = "../tests/support/tnef_recurrence.rs"]
mod recurrence_fixtures;

#[path = "support/tnef_calendar_recurrence.rs"]
mod calendar_recurrence_workloads;

#[path = "../tests/support/tnef_calendar.rs"]
mod calendar_fixtures;

#[path = "support/tnef_calendar_builder.rs"]
mod calendar_builder_workloads;

#[path = "support/tnef_bodies.rs"]
mod body_workloads;

#[path = "support/tnef_attachments.rs"]
mod attachment_workloads;

#[path = "support/tnef_messages.rs"]
mod message_workloads;

fn measure(name: &str, size: usize, mut read: impl FnMut()) {
    for _ in 0..3 {
        read();
    }
    let mut samples = [0.0f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        let mut count = 0;
        while start.elapsed() < Duration::from_millis(100) {
            read();
            count += 1;
        }
        *sample = count as f64 / start.elapsed().as_secs_f64();
    }
    samples.sort_by(f64::total_cmp);
    if size == 0 {
        println!("{name:28} {:10.1} batches/s (median of 5)", samples[2]);
    } else {
        println!(
            "{name:28} {:9.1} MiB/s {:10.1} documents/s (median of 5)",
            samples[2] * size as f64 / 1048576.0,
            samples[2]
        );
    }
}

fn main() {
    println!(
        "Calendar builder: 8 build/write/observe/drop operations per batch; source loading is excluded."
    );
    for name in calendar_builder_workloads::CASES {
        let case = calendar_builder_workloads::workload(name);
        case.check();
        measure(&format!("CalendarBuilder/{}", case.name), 0, || {
            black_box(calendar_builder_workloads::run(&case));
        });
    }
    println!(
        "Calendar recurrence: 64 generations per batch, including full rule/date/error observation and drop; parsing and zone/fixture construction are excluded."
    );
    for name in calendar_recurrence_workloads::CASES {
        let case = calendar_recurrence_workloads::workload(name);
        case.check();
        measure(&format!("CalendarRecurrence/{}", case.name), 0, || {
            black_box(calendar_recurrence_workloads::run(&case));
        });
    }
    println!(
        "Recurrence: 64 parses per batch, including complete field/text observation, diagnostics, and drop; fixture creation and independent expectations are excluded."
    );
    for name in recurrence_workloads::CASES {
        let case = recurrence_workloads::workload(name);
        case.check();
        measure(&format!("Recurrence/{}", case.name), case.bytes, || {
            black_box(recurrence_workloads::run(&case));
        });
    }
    println!(
        "Time zones: parse 256 structures, convert 512 prepared dates, or write 128 components per batch; includes result observation and drop."
    );
    println!(
        "Conversion batches exclude date/zone construction and have no byte throughput. Writing includes a fresh sink, finish/flush, and optional seven-byte writes."
    );
    for name in time_zone_workloads::CASES {
        let case = time_zone_workloads::workload(name);
        case.check();
        measure(&format!("TimeZone/{}", case.name), case.bytes, || {
            black_box(time_zone_workloads::run(&case));
        });
    }
    println!(
        "Calendar writing includes a fresh writer/sink, escaping, UTF-8 folding, finalization, full-output observation, and drop."
    );
    println!(
        "Formatting includes date/offset formatting; ShortWrites uses seven-byte I/O. Input and independent expected output are prepared outside measurement."
    );
    for name in calendar_workloads::CASES {
        let case = calendar_workloads::workload(name);
        case.check();
        measure(
            &format!("Calendar/Write/{}", case.name),
            case.output_length,
            || {
                black_box(calendar_workloads::write(&case));
            },
        );
    }
    println!(
        "TNEF writing includes a fresh writer, growing sink, framing/checksums, finalization, output observation, and drop."
    );
    println!(
        "Throughput counts complete serialized output bytes once; Stream uses a 4 KiB buffer, ShortReads/ShortWrites seven-byte I/O."
    );
    println!(
        "Buffered/property cases include fresh scratch storage and replay; BufferedShortChunks uses one-byte input, BufferedShortReads/Writes seven-byte scratch I/O."
    );
    println!(
        "PropertyStream cases append directly to attribute scratch; ShortChunks uses one-byte input, ShortWrites seven-byte scratch writes."
    );
    println!(
        "RtfProperty cases encode LZFu/MELA in attribute scratch; raw input preparation and decoding checks are outside measurement."
    );
    println!(
        "Embedded cases include nested framing and per-level scratch replay; fixture preparation and recursive reader checks are outside measurement."
    );
    for name in writer_workloads::CASES {
        let case = writer_workloads::workload(name);
        case.check();
        measure(
            &format!("TnefWriter/Write/{}", case.name),
            case.output_length,
            || {
                black_box(writer_workloads::write(&case));
            },
        );
    }
    println!(
        "RTF writing includes a fresh encoder/sink, headers, CRC, finalization, and output drop."
    );
    println!(
        "Throughput counts raw input bytes; ShortChunks uses one-byte input and ShortWrites a seven-byte sink."
    );
    for name in rtf_writer_workloads::CASES {
        let case = rtf_writer_workloads::workload(name);
        case.check();
        measure(
            &format!("RtfCompressed/Encode/{}", case.name),
            case.input.len(),
            || {
                black_box(rtf_writer_workloads::write(&case));
            },
        );
    }
    println!(
        "Whole messages: legacy/MAPI selection, recipient/attachment assembly, and final charsets."
    );
    println!(
        "Traverse also streams decoded bodies/content and recursively loads embedded messages."
    );
    println!(
        "Both include fresh readers, model allocation/drop, checksums, and a rejecting logger."
    );
    let cases: Vec<_> = message_workloads::CASES
        .into_iter()
        .map(message_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in message_workloads::MODES {
            measure(
                &format!("Messages/{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(message_workloads::read(case, mode));
                },
            );
        }
    }
    println!("Initial synchronous raw TNEF attribute baseline; no upstream TNEF benchmark exists.");
    println!("Fresh readers, 4 KiB read-ahead; Stream uses a reused 4 KiB stack buffer.");
    println!("Collect includes owned-value allocation/destruction. Skip still computes checksums.");
    println!("Throughput counts original TNEF bytes; setup and correctness checks are not timed.");
    println!("MAPI parsing, conversion, logging, async and sans-I/O are not measured.");
    let cases: Vec<_> = workloads::CASES
        .into_iter()
        .map(workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in workloads::MODES {
            measure(
                &format!("{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(workloads::read(case, mode));
                },
            );
        }
    }
    println!(
        "MAPI properties: generated fixed, multivalue, named, recipient, and binary workloads."
    );
    println!(
        "Skip navigates properties; Stream reads raw variable values; Decode collects native values."
    );
    println!(
        "Property collects/drops each snapshot; Set retains complete property sets or all recipient rows."
    );
    println!(
        "All modes parse names and checksums. No message/attachment models, embedded readers, logging, or MIME conversion."
    );
    let cases: Vec<_> = property_workloads::CASES
        .into_iter()
        .map(property_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in property_workloads::MODES {
            measure(
                &format!("Properties/{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(property_workloads::read(case, mode));
                },
            );
        }
    }
    println!("Date interpretation: legacy attributes, AppTime, FILETIME, and mixed multivalues.");
    println!(
        "Includes date validation, typed conversion, checksums, and a logger rejecting unexpected issues."
    );
    let cases: Vec<_> = date_workloads::CASES
        .into_iter()
        .map(date_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        measure(
            &format!("Dates/Read/{}", case.name),
            case.input.len(),
            || {
                black_box(date_workloads::read(case));
            },
        );
    }
    println!("Embedded messages: bounded recursive readers, inherited logger, and shared limits.");
    println!("Includes streaming bodies, checksums at every depth, and reader setup/destruction.");
    println!(
        "Detached also collects/drops owned content at every depth, with fresh child budgets."
    );
    let cases: Vec<_> = embedded_workloads::CASES
        .into_iter()
        .map(embedded_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        measure(
            &format!("Embedded/Read/{}", case.name),
            case.input.len(),
            || {
                black_box(embedded_workloads::read(case));
            },
        );
        measure(
            &format!("Embedded/Detached/{}", case.name),
            case.input.len(),
            || {
                black_box(embedded_workloads::read_detached(case));
            },
        );
    }
    println!("Owned access: preloaded snapshots; reader/setup/drop are outside measurement.");
    println!(
        "Equivalent wire-byte throughput, NOT parser throughput; includes binary text decoding."
    );
    let cases: Vec<_> = conversion_workloads::CASES
        .into_iter()
        .map(conversion_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        measure(
            &format!("Conversions/Access/{}", case.name),
            case.input_length,
            || {
                black_box(conversion_workloads::read(case));
            },
        );
    }
    println!("Recipients: 256 owned rows, type flags, name/address fallbacks, and binary text.");
    println!("Load includes parsing, checksums, model allocation/drop, and all four accessors.");
    println!(
        "Access uses preloaded rows; its wire-byte throughput is equivalent, NOT parser throughput."
    );
    let cases: Vec<_> = recipient_workloads::CASES
        .into_iter()
        .map(recipient_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in recipient_workloads::MODES {
            measure(
                &format!("Recipients/{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(recipient_workloads::read(case, mode));
                },
            );
        }
    }
    println!("Compressed RTF: literals, dictionary overlap/wrap, MELA, and one-byte input.");
    println!(
        "Includes fresh inline dictionary, bounded-chunk output buffers, CRC validation and drop."
    );
    let cases: Vec<_> = rtf_compressed_workloads::CASES
        .into_iter()
        .map(rtf_compressed_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        measure(
            &format!("RtfCompressed/Decode/{}", case.name),
            case.input.len(),
            || {
                black_box(rtf_compressed_workloads::read(case));
            },
        );
    }
    println!("Attachments: Load parses generated property sets and collects/drops content.");
    println!(
        "Access reads preloaded metadata; Stream reads preloaded raw content, excluding IIDs."
    );
    println!("One attachment per generated attribute; no whole-message loader or MIME conversion.");
    let cases: Vec<_> = attachment_workloads::CASES
        .into_iter()
        .map(attachment_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in attachment_workloads::MODES {
            measure(
                &format!("Attachments/{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(attachment_workloads::read(case, mode));
                },
            );
        }
    }
    println!("Bodies: Construct clones raw bytes and trims text terminators; no TNEF parsing.");
    println!("Stream/Text use preloaded bodies; RTF includes decompression and EOF validation.");
    println!("Throughput counts original raw body bytes, not decoded output.");
    let cases: Vec<_> = body_workloads::CASES
        .into_iter()
        .map(body_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in body_workloads::MODES {
            measure(
                &format!("Bodies/{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(body_workloads::read(case, mode));
                },
            );
        }
    }
}

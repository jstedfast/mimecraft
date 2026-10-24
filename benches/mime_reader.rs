// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    io::{BufRead, BufReader},
    time::{Duration, Instant},
};

use mimecraft::{MimeEvent, MimeParser, MimeReader};

fn measure(name: &str, bytes: usize, mut operation: impl FnMut()) -> f64 {
    for _ in 0..3 {
        operation();
    }
    let mut samples = [0.0f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        let mut iterations = 0;
        while start.elapsed() < Duration::from_millis(100) {
            operation();
            iterations += 1;
        }
        *sample = bytes as f64 * iterations as f64 / start.elapsed().as_secs_f64() / 1048576.0;
    }
    samples.sort_by(f64::total_cmp);
    println!("{name:42} {:9.1} MiB/s (median of 5)", samples[2]);
    samples[2]
}

fn scan(source: impl BufRead, expected: usize) {
    let mut bytes = 0;
    MimeReader::new(source)
        .read_message(|event| {
            if let MimeEvent::Content { bytes: content, .. } = event {
                black_box(content);
                bytes += content.len();
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(bytes, expected);
}

fn main() {
    println!(
        "target: {}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    println!(
        "Input-byte throughput; fixtures built outside timing; readers/models constructed per iteration."
    );
    let wrapped =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/AAAAAAAAAAAA\r\n"
            .repeat(14000);
    let long = b"0123456789abcdef".repeat(65536);
    for (name, body) in [("wrapped-base64", wrapped), ("long-line", long)] {
        let mut wire = b"Subject: benchmark\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\n".to_vec();
        wire.extend_from_slice(&body);
        measure(&format!("reader/{name}/slice"), wire.len(), || {
            scan(black_box(wire.as_slice()), body.len())
        });
        for capacity in [8192, 65536] {
            measure(
                &format!("reader/{name}/buffer={capacity}"),
                wire.len(),
                || {
                    scan(
                        BufReader::with_capacity(capacity, black_box(wire.as_slice())),
                        body.len(),
                    );
                },
            );
        }
        measure(&format!("parser/{name}/slice"), wire.len(), || {
            let message = MimeParser::new(black_box(wire.as_slice()))
                .parse_message()
                .unwrap();
            assert_eq!(
                message
                    .leaf_parts()
                    .next()
                    .unwrap()
                    .content()
                    .get_ref()
                    .get_ref()
                    .len(),
                body.len()
            );
            black_box(message);
        });
    }
    let attachment =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/\r\n".repeat(8192);
    let multipart = [
        b"Subject: attachment\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhello\r\n--b\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\n".as_slice(),
        &attachment,
        b"\r\n--b--\r\n",
    ].concat();
    measure("reader/multipart/slice", multipart.len(), || {
        scan(black_box(multipart.as_slice()), attachment.len() + 5)
    });
    let small = b"Subject: small\nFrom: a@example.com\nTo: b@example.com\nContent-Type: text/plain\n\nhello\n".to_vec();
    for (name, message, count) in [("small", small, 1024), ("multipart", multipart, 8)] {
        let record = [b"From sender@example.com\n".as_slice(), &message, b"\n"].concat();
        let archive = record.repeat(count);
        let rate = measure(&format!("reader/mbox-{name}/slice"), archive.len(), || {
            let mut reader = MimeReader::new(black_box(archive.as_slice()));
            let mut messages = 0;
            while reader
                .read_mbox_message(|event| {
                    black_box(event);
                    Ok(())
                })
                .unwrap()
            {
                messages += 1;
            }
            assert_eq!(messages, count);
        });
        println!(
            "  {name}: {:.0} messages/s",
            rate * 1048576.0 * count as f64 / archive.len() as f64
        );
        measure(&format!("parser/mbox-{name}/slice"), archive.len(), || {
            let mut parser = MimeParser::new(black_box(archive.as_slice()));
            let mut messages = 0;
            while let Some(message) = parser.parse_mbox_message().unwrap() {
                black_box(message);
                messages += 1;
            }
            assert_eq!(messages, count);
        });
    }
}

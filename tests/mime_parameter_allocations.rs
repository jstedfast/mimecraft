// Adapted from MimeKit's MimeComplianceParameterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    fmt::Write,
    time::{Duration, Instant},
};

use mimecraft::{
    MimeComplianceIssue, MimeComplianceOptions, MimeCompliancePositionKind,
    MimeComplianceViolation as Violation, MimeEntityParseOptions, MimeReader,
};

struct MeasuredAllocator;

thread_local! {
    static ALLOCATED: Cell<Option<usize>> = const { Cell::new(None) };
}

fn record(size: usize) {
    // TLS may be unavailable during thread teardown, outside any measurement.
    let _ = ALLOCATED.try_with(|allocated| {
        if let Some(bytes) = allocated.get() {
            allocated.set(Some(bytes + size));
        }
    });
}

// SAFETY: All allocation operations preserve System's pointer/layout contract.
// Accounting is thread-local, does not allocate, and never touches the memory.
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a System allocation and its original layout.
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() {
            record(size);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The pointer and layout are forwarded unchanged to their allocator.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

fn measure_headers(text: &str) -> (Vec<MimeComplianceIssue>, usize, Duration) {
    let mut reader = MimeReader::with_options(
        text.as_bytes(),
        MimeEntityParseOptions {
            max_header_length: text.len(),
            ..Default::default()
        },
    )
    .unwrap();
    reader.set_compliance_options(MimeComplianceOptions {
        max_issues_per_violation: 0,
        ..Default::default()
    });
    let mut issues = Vec::new();
    // Unlike the upstream private-validator measurement, this also includes
    // raw header parsing and reporting setup, under the same allocation cap.
    let start = Instant::now();
    ALLOCATED.with(|allocated| allocated.set(Some(0)));
    let result = reader.read_headers_with_logger(
        &mut |issue: MimeComplianceIssue| {
            issues.push(issue);
            Ok(())
        },
        |_| Ok(()),
    );
    let allocated = ALLOCATED.with(|allocated| allocated.replace(None).unwrap());
    let elapsed = start.elapsed();
    result.unwrap();
    (issues, allocated, elapsed)
}

#[test]
fn many_malformed_parameters_are_bounded() {
    const COUNT: usize = 16_384;
    for parameter in ["broken", "x="] {
        let text = format!(
            "Content-Disposition: attachment{}\r\n\r\n",
            format!(";\r\n {parameter}").repeat(COUNT)
        );
        let (issues, allocated, elapsed) = measure_headers(&text);
        let invalid: Vec<_> = issues
            .iter()
            .filter(|i| i.violation == Violation::InvalidParameter)
            .collect();
        assert_eq!(invalid.len(), COUNT);
        assert!(
            invalid
                .iter()
                .all(|i| i.position_kind == MimeCompliancePositionKind::ElementStart)
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.violation == Violation::RepeatedParameter)
                .count(),
            if parameter == "x=" { COUNT - 1 } else { 0 },
        );
        let last = issues.last().unwrap();
        assert_eq!(last.line_number, COUNT as u64 + 1);
        assert_eq!(last.column_number, 2);
        assert_eq!(
            last.stream_offset,
            (text.len() - parameter.len() - 4) as u64
        );
        assert!(elapsed < Duration::from_secs(5), "{parameter}: {elapsed:?}");
        assert!(
            allocated < COUNT * 256 + 65536,
            "{parameter}: {allocated} bytes"
        );
    }
}

#[test]
fn segment_tracking_is_bounded_by_declaration_count() {
    const COUNT: usize = 4096;
    for sparse in [false, true] {
        let mut text = String::from("Content-Disposition: attachment");
        let mut last = 0;
        for index in 0..COUNT {
            last = if sparse {
                i32::MAX as usize - index * 9059
            } else {
                COUNT - index - 1
            };
            write!(text, "; filename*{last}=x").unwrap();
        }
        write!(text, "; filename*{last}=y\r\n\r\n").unwrap();
        let (issues, allocated, elapsed) = measure_headers(&text);

        assert_eq!(
            issues
                .iter()
                .filter(|i| i.violation == Violation::RepeatedFilenameParameter)
                .count(),
            1
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.violation == Violation::MissingParameterSegment)
                .count(),
            usize::from(sparse)
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "sparse={sparse}: {elapsed:?}"
        );
        assert!(
            allocated < COUNT * 256 + 65536,
            "sparse={sparse}: {allocated} bytes"
        );
    }
}

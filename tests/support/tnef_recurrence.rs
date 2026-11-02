// Adapted from MimeKit's TnefCalendarTests and TnefCalendarRobustnessTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

pub fn put16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub fn put32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// The master portion only. AppointmentRecurrencePattern starts on 2024-07-01
/// at 10:00 local time, ending after ten occurrences unless a test changes it.
pub fn master(pattern: u16, period: u32, specific: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [0x3004, 0x3004, 0x200b, pattern, 0] {
        put16(&mut bytes, value);
    }
    for value in [0, period, 0] {
        put32(&mut bytes, value);
    }
    for &value in specific {
        put32(&mut bytes, value);
    }
    for value in [
        0x2022, 10, 0, 0, 0, 222737760, 222780960, 0x3006, 0x3009, 600, 660,
    ] {
        put32(&mut bytes, value);
    }
    bytes
}

pub fn pattern(pattern: u16, period: u32, specific: &[u32]) -> Vec<u8> {
    let mut bytes = master(pattern, period, specific);
    put16(&mut bytes, 0); // ExceptionCount
    put32(&mut bytes, 0); // ReservedBlock1Size
    bytes
}

// Weekly Monday/Wednesday at 10:00: July 8 deleted, July 10 moved to 13:00,
// with a new subject and location. Numeric dates are independent .NET values.
pub fn weekly() -> Vec<u8> {
    let mut bytes = master(1, 1, &[0x0a]);
    let mut arrays = Vec::new();
    for value in [2, 222747840, 222750720, 1, 222750720] {
        put32(&mut arrays, value);
    }
    bytes.splice(38..46, arrays);
    put16(&mut bytes, 1);
    for value in [222751500, 222751560, 222751320] {
        put32(&mut bytes, value);
    }
    put16(&mut bytes, 0x0011); // Subject | Location
    put16(&mut bytes, 11);
    put16(&mut bytes, 10);
    bytes.extend_from_slice(b"Moved sync");
    put16(&mut bytes, 7);
    put16(&mut bytes, 6);
    bytes.extend_from_slice(b"Room 2");
    put32(&mut bytes, 0); // ReservedBlock1Size
    put32(&mut bytes, 4); // ChangeHighlightSize
    put32(&mut bytes, 0); // ChangeHighlightValue
    put32(&mut bytes, 0); // ReservedBlockEE1Size
    for value in [222751500, 222751560, 222751320] {
        put32(&mut bytes, value);
    }
    put16(&mut bytes, 10);
    bytes.extend("Moved s\u{fd}nc".encode_utf16().flat_map(u16::to_le_bytes));
    put16(&mut bytes, 6);
    bytes.extend("Room 2".encode_utf16().flat_map(u16::to_le_bytes));
    put32(&mut bytes, 0); // ReservedBlockEE2Size
    put32(&mut bytes, 0); // ReservedBlock2Size (ignored)
    bytes
}

// Weekly every day: the first `count` instances move from 10:00 to 13:00.
pub fn many(count: u16) -> Vec<u8> {
    let mut bytes = master(1, 1, &[0x7f]);
    bytes[30..34].copy_from_slice(&(u32::from(count) + 10).to_le_bytes());
    let mut arrays = Vec::new();
    for _ in 0..2 {
        put32(&mut arrays, u32::from(count));
        for index in 0..u32::from(count) {
            put32(&mut arrays, 222737760 + index * 1440);
        }
    }
    bytes.splice(38..46, arrays);
    put16(&mut bytes, count);
    for index in 0..u32::from(count) {
        let day = 222737760 + index * 1440;
        for value in [day + 780, day + 840, day + 600] {
            put32(&mut bytes, value);
        }
        put16(&mut bytes, 0);
    }
    put32(&mut bytes, 0); // ReservedBlock1Size
    for _ in 0..count {
        for value in [4, 0, 0] {
            put32(&mut bytes, value);
        }
    }
    put32(&mut bytes, 0); // ReservedBlock2Size
    bytes
}

// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    time::{Duration, Instant},
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
#[allow(unsafe_code)] // GlobalAlloc requires unsafe forwarding in this test-only allocator.
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

struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        ALLOCATED.with(|allocated| allocated.set(None));
    }
}

pub fn measure<T>(action: impl FnOnce() -> T) -> (T, usize, Duration) {
    let start = Instant::now();
    ALLOCATED.with(|allocated| {
        assert!(
            allocated.get().is_none(),
            "allocation measurements cannot be nested"
        );
        allocated.set(Some(0));
    });
    let _reset = Reset;
    let result = action();
    let allocated = ALLOCATED.with(|allocated| allocated.get().unwrap());
    (result, allocated, start.elapsed())
}

//! Experiment only (branch exp/crossfile-diagnostics): per-thread cumulative counters for attributing checker work to
//! diagnostics filed against files the checker does not check. Compiled in with `--features xfile-stats`; otherwise
//! every function is a no-op that returns zeros.

use std::cell::Cell;

pub const COMPILED: bool = cfg!(feature = "xfile-stats");

thread_local! {
    static ARENA_BYTES: Cell<u64> = const { Cell::new(0) };
    static HEAP_BYTES: Cell<u64> = const { Cell::new(0) };
}

#[inline(always)]
pub fn arena_add(n: usize) {
    if COMPILED {
        let _ = ARENA_BYTES.try_with(|c| c.set(c.get() + n as u64));
    }
}

#[inline(always)]
pub fn heap_add(n: usize) {
    if COMPILED {
        let _ = HEAP_BYTES.try_with(|c| c.set(c.get() + n as u64));
    }
}

#[cfg(target_os = "macos")]
fn instructions() -> u64 {
    extern "C" {
        fn thread_selfcounts(kind: i32, buf: *mut u64, nbytes: usize) -> i32;
    }
    let mut b = [0u64; 2];
    // SAFETY: writes two u64 (instructions, cycles) of the calling thread into the buffer.
    if unsafe { thread_selfcounts(1, b.as_mut_ptr(), 16) } != 0 {
        return 0;
    }
    b[0]
}

#[cfg(not(target_os = "macos"))]
fn instructions() -> u64 {
    0
}

/// (arena bytes allocated, heap bytes allocated, instructions retired) by the calling thread so far.
#[inline]
pub fn snapshot() -> [u64; 3] {
    if !COMPILED {
        return [0; 3];
    }
    [ARENA_BYTES.try_with(Cell::get).unwrap_or(0), HEAP_BYTES.try_with(Cell::get).unwrap_or(0), instructions()]
}

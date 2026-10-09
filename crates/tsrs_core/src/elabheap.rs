//! exp/discarded-elaboration: per-thread heap byte counters, fed by the `elab-heap` allocator of tsrs_cli.
use std::cell::Cell;

thread_local! {
    static ALLOC: Cell<u64> = const { Cell::new(0) };
    static FREE: Cell<u64> = const { Cell::new(0) };
}

#[inline]
pub fn note_alloc(n: usize) {
    let _ = ALLOC.try_with(|c| c.set(c.get() + n as u64));
}

#[inline]
pub fn note_free(n: usize) {
    let _ = FREE.try_with(|c| c.set(c.get() + n as u64));
}

/// (bytes allocated, bytes freed) by this thread so far.
pub fn counters() -> (u64, u64) {
    (ALLOC.try_with(Cell::get).unwrap_or(0), FREE.try_with(Cell::get).unwrap_or(0))
}

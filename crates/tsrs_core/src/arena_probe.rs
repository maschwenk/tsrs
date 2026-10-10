//! The "no-malloc ceiling" probe (`--features arena-probe` on `tsrs_cli`; notes/exp-arena-probe.md): a global
//! allocator that, while a thread is in arena mode (`enter_arena_mode`, the checker threads), serves every heap
//! allocation of at most `TSRS_ARENA_PROBE_MAX` bytes (default 512, `all` for everything) from the thread's
//! current arena instead of mimalloc: a block of the size's free class (`arena::free_class`) if the list has one,
//! else a bump. A free of a block in the reservation goes to that free list when the current arena's current chunk
//! holds it and the size has a class, else the block leaks; a block born on mimalloc stays there. The counters print on stderr at exit (`dump`). A measurement, not a design: it
//! bounds what arena-backed containers could gain in instructions, and what they cost in memory without larger
//! free classes.

#[cfg(not(compressed_ptrs))]
compile_error!("arena-probe needs compressed pointers: the reservation test tells arena blocks from heap blocks");
#[cfg(feature = "alloc-profile")]
compile_error!("arena-probe and alloc-profile both install a global allocator; build with one of them");

use crate::arena::{self, free_class, Arena};
use crate::reserve::{BASE_ADDR, RESERVE};
use mimalloc_safe::MiMalloc;
use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Routes allocations of arena-mode threads to their arena (see the module docs).
pub struct ArenaRouting;

#[global_allocator]
static GLOBAL: ArenaRouting = ArenaRouting;

/// `TSRS_ARENA_PROBE_MAX`: largest routed allocation; 0 until `enter_arena_mode` parses it.
static PROBE_MAX: AtomicUsize = AtomicUsize::new(0);

const ROUTED_ALLOCS: usize = 0;
const ROUTED_BYTES: usize = 1;
const RECYCLED_FREES: usize = 2;
const LEAKED_FREES: usize = 3;
const LEAKED_BYTES: usize = 4;
const HEAP_ALLOCS: usize = 5;
const COUNTERS: usize = 6;
const NAMES: [&str; COUNTERS] = ["routed_allocs", "routed_bytes", "recycled_frees", "leaked_frees", "leaked_bytes", "heap_allocs"];

static TOTALS: [AtomicU64; COUNTERS] = [const { AtomicU64::new(0) }; COUNTERS];

/// The probe's state, a field of every `Arena` (`Arena::probe`): one arena has one owner thread at a time, so a flag
/// on the thread's current arena is a thread flag that costs no thread-local of its own (on macOS every
/// `thread_local!` is a `tlv_get_addr` call; `arena::CURRENT` is the one lookup a real arena allocator pays too).
pub(crate) struct ArenaState {
    /// Heap allocations of the owner thread go to this arena.
    mode: Cell<bool>,
    /// Inside the probe's own call into the arena: its bookkeeping (a retired-chunk push, a new chunk's registry
    /// entry) must not come back here.
    internals: Cell<bool>,
    /// Counters, added to `TOTALS` when the owner leaves arena mode and by `dump`.
    counts: [Cell<u64>; COUNTERS],
}

impl ArenaState {
    pub(crate) const fn new() -> ArenaState {
        ArenaState { mode: Cell::new(false), internals: Cell::new(false), counts: [const { Cell::new(0) }; COUNTERS] }
    }

    #[inline]
    fn count(&self, which: usize, n: u64) {
        self.counts[which].set(self.counts[which].get() + n);
    }

    fn flush(&self) {
        for (total, local) in TOTALS.iter().zip(&self.counts) {
            // Relaxed: counters summed at exit, after every thread joined.
            total.fetch_add(local.take(), Ordering::Relaxed);
        }
    }
}

/// Arena mode for the calling thread's current arena until the guard is dropped.
pub struct ProbeGuard {
    arena: &'static Arena,
    prev: bool,
}

/// Puts the calling thread's current arena in arena mode. Reads `TSRS_ARENA_PROBE_MAX` the first time (a number of
/// bytes, or `all`).
pub fn enter_arena_mode() -> ProbeGuard {
    // Relaxed: set before this thread's first routed allocation; every arena-mode thread passes here first.
    if PROBE_MAX.load(Ordering::Relaxed) == 0 {
        let max = match std::env::var("TSRS_ARENA_PROBE_MAX").ok().as_deref() {
            None => 512,
            Some("all") => usize::MAX,
            Some(s) => s.parse().expect("TSRS_ARENA_PROBE_MAX: a size in bytes or `all`"),
        };
        // Relaxed: see above.
        PROBE_MAX.store(max.max(1), Ordering::Relaxed);
    }
    let arena = crate::ptr::with_arena(|a| a);
    ProbeGuard { arena, prev: arena.probe.mode.replace(true) }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        self.arena.probe.mode.set(self.prev);
        if !self.prev {
            self.arena.probe.flush();
        }
    }
}

/// Prints the counters on stderr (`arena-probe: <name> <value>`, one per line).
pub fn dump() {
    if let Some(a) = current() {
        a.probe.flush();
    }
    for (name, total) in NAMES.iter().zip(&TOTALS) {
        // Relaxed: read after every arena-mode thread joined.
        eprintln!("arena-probe: {name} {}", total.load(Ordering::Relaxed));
    }
}

#[inline]
fn in_reservation(addr: usize) -> bool {
    addr.wrapping_sub(BASE_ADDR) < RESERVE
}

/// The block's layout in the arena: sizes rounded to 8 so a later free lands in an exact free class, 8-aligned so
/// `free_block` accepts the address (`P` targets are laid out the same way, `ptr::p_layout`).
#[inline]
fn arena_layout(layout: Layout) -> (usize, usize) {
    (layout.size().next_multiple_of(8), layout.align().max(8))
}

/// The calling thread's current allocation target, if it has one.
#[inline]
fn current() -> Option<&'static Arena> {
    let p = arena::CURRENT.try_with(Cell::get).unwrap_or(std::ptr::null());
    // SAFETY: a non-null `CURRENT` is the thread arena or the arena of a region kept alive by an entered scope.
    (!p.is_null()).then(|| unsafe { &*p })
}

/// The arena block for `layout` when it is routed (the current arena is in arena mode, the probe is not inside it,
/// the size is at most `PROBE_MAX`): a block of its free class if the list has one, else a bump. `None`: mimalloc.
#[inline]
fn arena_alloc(layout: Layout) -> Option<*mut u8> {
    let a = current()?;
    let s = &a.probe;
    // Relaxed: see `enter_arena_mode`.
    if !s.mode.get() || s.internals.get() || layout.size() > PROBE_MAX.load(Ordering::Relaxed) {
        if s.mode.get() && !s.internals.get() {
            s.count(HEAP_ALLOCS, 1);
        }
        return None;
    }
    s.internals.set(true);
    let (size, align) = arena_layout(layout);
    let class = free_class(size, align);
    let p = match (class != 0).then(|| a.pop_free(class)).flatten() {
        Some(p) => p,
        // SAFETY: `size` is a multiple of 8 near `layout.size()` (no overflow for a valid layout); `align` is a power of two.
        None => a.alloc_layout(unsafe { Layout::from_size_align_unchecked(size, align) }),
    };
    s.internals.set(false);
    s.count(ROUTED_ALLOCS, 1);
    s.count(ROUTED_BYTES, size as u64);
    Some(p.as_ptr())
}

/// A free of a block in the reservation: recycled into the current arena when its current chunk holds the block and
/// the size has a free class, else leaked. Regardless of arena mode (a block born in arena mode may die on any thread).
#[inline]
fn arena_free(addr: usize, layout: Layout) {
    let Some(a) = current() else {
        return; // a thread without an arena leaks the block (uncounted)
    };
    let s = &a.probe;
    let (size, align) = arena_layout(layout);
    if free_class(size, align) != 0 && a.owns_current_chunk(addr) {
        s.internals.set(true);
        // SAFETY: the global allocator's caller gives up the block (`dealloc`'s contract); `addr` lies in the
        // current chunk of the arena that is the thread's target, so it is the one that handed the block out.
        unsafe { arena::free_block(a, addr, size, align, false) };
        s.internals.set(false);
        s.count(RECYCLED_FREES, 1);
        return;
    }
    s.count(LEAKED_FREES, 1);
    s.count(LEAKED_BYTES, size as u64);
}

// SAFETY: every block comes from mimalloc or from an arena chunk of the reservation and is told apart by its
// address; both are valid for the block's whole life (arena chunks are never released while a thread arena is
// alive, and a recycled block is handed out again only by `pop_free`). `realloc` copies and frees by the same rule.
unsafe impl GlobalAlloc for ArenaRouting {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if let Some(p) = arena_alloc(layout) {
            return p;
        }
        // SAFETY: the caller's `layout` is valid (`GlobalAlloc`'s contract).
        unsafe { MiMalloc.alloc(layout) }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if let Some(p) = arena_alloc(layout) {
            // SAFETY: a fresh block of at least `layout.size()` bytes; recycled and rewound memory is not zero.
            unsafe { std::ptr::write_bytes(p, 0, layout.size()) };
            return p;
        }
        // SAFETY: as in `alloc`.
        unsafe { MiMalloc.alloc_zeroed(layout) }
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if in_reservation(p.addr()) {
            return arena_free(p.addr(), layout);
        }
        // SAFETY: `p` is not in the reservation, so mimalloc handed it out (`dealloc`'s contract otherwise).
        unsafe { MiMalloc.dealloc(p, layout) }
    }

    #[inline]
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if !in_reservation(p.addr()) {
            // Born on mimalloc, stays there (no copy into the arena).
            // SAFETY: `p` is a live mimalloc block of `layout` (`realloc`'s contract).
            return unsafe { MiMalloc.realloc(p, layout, new_size) };
        }
        // SAFETY: `new_size` is non-zero and does not overflow with `layout.align()` (`realloc`'s contract).
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        // SAFETY: `new_layout` is valid (above).
        let q = unsafe { self.alloc(new_layout) };
        if !q.is_null() {
            // SAFETY: `p` holds `layout.size()` live bytes, `q` is a fresh block of `new_size`; distinct blocks.
            unsafe { std::ptr::copy_nonoverlapping(p, q, layout.size().min(new_size)) };
            // SAFETY: as in `dealloc`.
            unsafe { self.dealloc(p, layout) };
        }
        q
    }
}

//! tsrs-only: the syntax tree's node rows as columns (notes/dod-ast-tables.md, after yuku's `Tree`: nodes in a
//! struct-of-arrays, children as `u32` indices).
//!
//! A node is a dense `u32` index. What used to be the 24-byte node header (crates/tsrs_ast/src/ast.rs before this
//! change: the parent, kind, data tag, flags, id and range in one allocation, with the node's data struct right after
//! it) lives in parallel column arrays indexed by that index: `kind`, `tag`, `flags`, `id`, `parent`, `data` (the
//! arena handle of the node's data struct, 0 for payload-less nodes) and `loc`. `P<Node>` stays the handle: `Node` is a
//! zero-sized view whose address encodes the index (`key_of` / `index_of`), so `P<Node>` keeps its identity, hashing,
//! ordering and `Option` niche, and every call site stays as it was.
//!
//! Indices are handed out the way the arena hands out bytes: a process-wide counter gives each thread a chunk
//! (`take_chunk`, doubling up to `MAX_CHUNK`), and the thread allocates from it without synchronization. A
//! speculative parse records the thread's position (`checkpoint`) and gives the indices it took back (`rewind`),
//! under the same rule the arena uses (the parser rewinds nodes only when it rewinds the arena). A file parsed into a
//! region of its own is a segment (`begin_segment` / `end_segment`): it allocates from the thread's chunks like any
//! other node and `end_segment` returns the exact ranges of rows it kept, so that their pages can be given back when
//! the file's region is freed (`discard`, fileregions.rs). Nothing is reserved for it: a reservation leaves a gap
//! after each file, and every column then keeps a partly used page per file (seven columns, where the arena's header
//! had one), which cost about 160 MiB of resident memory on the vscode bench project.
//!
//! Columns are written by the thread that creates the node, then read by every thread, like the cells of the old
//! header ("Threading" in docs/PORTING.md); the id column is atomic, as the id was (`get_node_id` races).
//!
//! Backing: on unix one fixed-address reservation holds every column (`COLUMNS_BASE`, like `reserve::BASE_ADDR`: a
//! constant base folds into the addressing mode), mapped read-write and never committed until touched, so untouched
//! index space costs nothing. Elsewhere (wasm, Windows) columns are chunked tables allocated on demand.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use crate::ptr::PKey;
use crate::TextRange;

/// Indices are below `1 << INDEX_BITS`: 512M nodes (the 38k-file codebase has 23M).
pub const INDEX_BITS: u32 = 29;
pub const MAX: usize = 1 << INDEX_BITS;

/// Bytes of columns per node (for sizing tools and notes).
pub const BYTES_PER_NODE: usize = 2 + 1 + 4 + 4 + std::mem::size_of::<PKey>() * 2 + std::mem::size_of::<TextRange>();

// Column numbers (`backing::col`).
const KIND: usize = 0;
const TAG: usize = 1;
const FLAGS: usize = 2;
const ID: usize = 3;
const PARENT: usize = 4;
const DATA: usize = 5;
const LOC: usize = 6;
const COLUMNS: usize = 7;

/// `P<Node>::key()` of the node at `idx`: the handle itself with compressed pointers (its "address" is
/// `reserve::BASE_ADDR + idx * 8`, inside the arena reservation, where no object is ever read through it: `Node` is
/// zero-sized), else the address `idx * 8` (non-null and 8-aligned, which is all a zero-sized value needs).
#[inline(always)]
#[expect(clippy::inline_always, reason = "every node field read goes through it")]
pub fn key_of(idx: u32) -> PKey {
    #[cfg(compressed_ptrs)]
    return idx;
    #[cfg(not(compressed_ptrs))]
    return (idx as PKey) << 3;
}

/// The index of the node whose zero-sized view is at `addr` (`key_of` inverted).
#[inline(always)]
#[expect(clippy::inline_always, reason = "every node field read goes through it")]
pub fn index_of(addr: usize) -> u32 {
    #[cfg(compressed_ptrs)]
    return (addr.wrapping_sub(crate::reserve::BASE_ADDR) >> 3) as u32;
    #[cfg(not(compressed_ptrs))]
    return (addr >> 3) as u32;
}

// ---------------------------------------------------------------------------------------------------------------
// Backing
// ---------------------------------------------------------------------------------------------------------------

#[cfg(unix)]
mod backing {
    use super::{COLUMNS, MAX};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Right above the arena reservation (`reserve::BASE_ADDR` + 32 GiB = 0x4009_0000_0000), at a 1 TiB mark.
    pub const COLUMNS_BASE: usize = 0x4010_0000_0000;
    /// Each column gets 4 GiB of address space: `MAX` rows of at most 8 bytes.
    pub const STRIDE: usize = 1 << 32;
    const _: () = assert!(MAX * 8 <= STRIDE);

    static RESERVED: AtomicBool = AtomicBool::new(false);

    /// The address of row `idx` of column `c` (a `T`).
    #[inline(always)]
    #[expect(clippy::inline_always, reason = "every node field read goes through it")]
    pub fn col<T>(c: usize, idx: u32) -> *mut T {
        std::ptr::with_exposed_provenance_mut(COLUMNS_BASE + c * STRIDE + idx as usize * std::mem::size_of::<T>())
    }

    /// Maps the columns' address range read-write, uncommitted (the first chunk take). Pages are committed on first
    /// touch, so the index space costs what the nodes use, rounded to pages.
    pub fn ensure_reserved() {
        // Acquire: pairs with the Release store in `reserve_slow`, so the mapping is visible before any column is touched.
        if RESERVED.load(Ordering::Acquire) {
            return;
        }
        reserve_slow();
    }

    #[cold]
    fn reserve_slow() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Acquire: as in `ensure_reserved`; the lock already orders the other threads' stores.
        if RESERVED.load(Ordering::Acquire) {
            return;
        }
        // SAFETY: a fresh anonymous mapping at a hint; nothing else refers to it.
        let p = unsafe {
            libc::mmap(
                std::ptr::without_provenance_mut(COLUMNS_BASE),
                COLUMNS * STRIDE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if p == libc::MAP_FAILED {
            fail("could not reserve the node column address range");
        }
        if p.addr() != COLUMNS_BASE {
            // SAFETY: the mapping just made, unused.
            unsafe { libc::munmap(p, COLUMNS * STRIDE) };
            fail("the node column address range is taken");
        }
        let _ = p.expose_provenance();
        // Release: publishes the mapping to `ensure_reserved`'s Acquire load.
        RESERVED.store(true, Ordering::Release);
    }

    /// Chunked backing hook: nothing to do, the reservation covers every index.
    #[inline]
    pub fn ensure_rows(_start: u32, _end: u32) {}

    /// Gives the whole pages of column `c`'s rows `start .. end` back to the system; the rows are never used again.
    ///
    /// # Safety
    /// Nothing may read or write those rows afterwards.
    pub unsafe fn discard_rows<T>(c: usize, start: u32, end: u32) {
        let p = col::<T>(c, start).cast::<u8>();
        let len = (end - start) as usize * std::mem::size_of::<T>();
        let page = page_size();
        let lo = p.addr().next_multiple_of(page);
        let hi = (p.addr() + len) & !(page - 1);
        if hi <= lo {
            return;
        }
        let q = p.with_addr(lo);
        // Linux: `MADV_DONTNEED` keeps one mapping (a stray read sees zeros). Elsewhere a no-access mapping over the
        // pages takes them out of the footprint at once (as `reserve::discard` does for arena pages).
        #[cfg(target_os = "linux")]
        // SAFETY: whole pages inside the columns' mapping that nothing uses any more (the caller's promise).
        unsafe {
            libc::madvise(q.cast(), hi - lo, libc::MADV_DONTNEED)
        };
        #[cfg(not(target_os = "linux"))]
        {
            // SAFETY: as above; a fixed no-access mapping over the pages replaces their contents and keeps the range.
            let r = unsafe {
                libc::mmap(q.cast(), hi - lo, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE | libc::MAP_FIXED, -1, 0)
            };
            if r == libc::MAP_FAILED {
                fail("could not give node column pages back");
            }
        }
    }

    fn page_size() -> usize {
        static PAGE_SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        // SAFETY: sysconf reads a system constant.
        *PAGE_SIZE.get_or_init(|| usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok().filter(|p| p.is_power_of_two()).unwrap_or(4096))
    }

    #[cold]
    fn fail(what: &str) -> ! {
        eprintln!("tsrs: {what} (node columns: one {} GiB address range for the syntax tree's rows)", (COLUMNS * STRIDE) >> 30);
        std::process::abort()
    }
}

#[cfg(not(unix))]
mod backing {
    use super::{COLUMNS, MAX};
    use std::sync::atomic::{AtomicPtr, Ordering};

    /// Rows per chunk.
    const CHUNK_SHIFT: u32 = 16;
    const CHUNKS: usize = MAX >> CHUNK_SHIFT;
    /// Widest column (`TextRange`, `usize` keys).
    const ROW_MAX: usize = 8;

    static TABLES: [[AtomicPtr<u8>; CHUNKS]; COLUMNS] = [const { [const { AtomicPtr::new(std::ptr::null_mut()) }; CHUNKS] }; COLUMNS];

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "every node field read goes through it")]
    pub fn col<T>(c: usize, idx: u32) -> *mut T {
        // Acquire: pairs with the Release in `ensure_rows`, which ran on the allocating thread before the node was
        // published to this one.
        let chunk = TABLES[c][(idx >> CHUNK_SHIFT) as usize].load(Ordering::Acquire);
        debug_assert!(!chunk.is_null(), "node column chunk not allocated");
        // SAFETY: `ensure_rows` allocated the chunk before any row of it was handed out.
        unsafe { chunk.add((idx as usize & ((1 << CHUNK_SHIFT) - 1)) * std::mem::size_of::<T>()).cast::<T>() }
    }

    pub fn ensure_reserved() {}

    /// Allocates (zeroed) the chunks of every column that rows `start .. end` lie in.
    pub fn ensure_rows(start: u32, end: u32) {
        for chunk in (start >> CHUNK_SHIFT)..=((end - 1) >> CHUNK_SHIFT) {
            for table in &TABLES {
                let slot = &table[chunk as usize];
                // Acquire: pairs with the Release in the compare_exchange below, as `col` does for readers.
                if !slot.load(Ordering::Acquire).is_null() {
                    continue;
                }
                let layout = std::alloc::Layout::from_size_align((1 << CHUNK_SHIFT) * ROW_MAX, 16).expect("node column chunk layout");
                // SAFETY: a non-zero-size layout.
                let p = unsafe { std::alloc::alloc_zeroed(layout) };
                assert!(!p.is_null(), "node columns: out of memory");
                // Release: readers of rows in the chunk Acquire it (`col`).
                if slot.compare_exchange(std::ptr::null_mut(), p, Ordering::Release, Ordering::Acquire).is_err() {
                    // SAFETY: just allocated, unused.
                    unsafe { std::alloc::dealloc(p, layout) };
                }
            }
        }
    }

    /// Nothing is given back without a page-granular mapping.
    pub unsafe fn discard_rows<T>(_c: usize, _start: u32, _end: u32) {}
}

// ---------------------------------------------------------------------------------------------------------------
// Index allocation
// ---------------------------------------------------------------------------------------------------------------

/// Index 0 is never a node (`Option<P<Node>>`'s niche is handle 0).
static NEXT: AtomicU32 = AtomicU32::new(1);

const FIRST_CHUNK: u32 = 1 << 10;
const MAX_CHUNK: u32 = 1 << 16;

/// A file region's segment: the thread's chunk when it began (`begin`, `begin_end`) and the chunks it took since.
struct Segment {
    begin: u32,
    begin_end: u32,
    chunks: Vec<(u32, u32)>,
}

thread_local! {
    /// The thread's current chunk: `(next, end)`.
    static CHUNK: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
    /// Size of the thread's next chunk (doubles).
    static CHUNK_SIZE: Cell<u32> = const { Cell::new(FIRST_CHUNK) };
    static SEGMENT: RefCell<Option<Segment>> = const { RefCell::new(None) };
}

#[cold]
fn exhausted() -> ! {
    eprintln!("tsrs: the node index space is exhausted ({} nodes)", MAX);
    std::process::abort()
}

/// Takes `n` fresh indices from the process-wide counter.
fn take_indices(n: u32) -> u32 {
    backing::ensure_reserved();
    // Relaxed: the counter only hands out disjoint ranges; nothing is published through it.
    let start = NEXT.fetch_add(n, Ordering::Relaxed);
    if start as usize + n as usize > MAX {
        exhausted();
    }
    backing::ensure_rows(start, start + n);
    start
}

#[cold]
#[inline(never)]
fn take_chunk() -> u32 {
    let size = CHUNK_SIZE.get();
    CHUNK_SIZE.set((size * 2).min(MAX_CHUNK));
    let start = take_indices(size);
    CHUNK.set((start + 1, start + size));
    SEGMENT.with_borrow_mut(|seg| {
        if let Some(seg) = seg {
            seg.chunks.push((start, start + size));
        }
    });
    start
}

/// Allocates a node row: `kind` and `tag` as stored by the caller, `data` the key of the node's data struct (0 for
/// none); flags, id and parent 0, range `loc`. Returns the index.
#[inline]
pub fn alloc(kind: u16, tag: u8, data: PKey, loc: TextRange) -> u32 {
    let (next, end) = CHUNK.get();
    let idx = if next < end {
        CHUNK.set((next + 1, end));
        next
    } else {
        take_chunk()
    };
    // SAFETY: a fresh row of a mapped (or allocated) chunk that no other thread knows yet.
    unsafe {
        backing::col::<u16>(KIND, idx).write(kind);
        backing::col::<u8>(TAG, idx).write(tag);
        backing::col::<u32>(FLAGS, idx).write(0);
        backing::col::<u32>(ID, idx).write(0);
        backing::col::<PKey>(PARENT, idx).write(0);
        backing::col::<PKey>(DATA, idx).write(data);
        backing::col::<TextRange>(LOC, idx).write(loc);
    }
    idx
}

/// The thread's allocation position (`rewind`).
#[derive(Clone, Copy)]
pub struct Checkpoint((u32, u32));

#[inline]
pub fn checkpoint() -> Checkpoint {
    Checkpoint(CHUNK.get())
}

/// Gives back every index this thread allocated since `cp`. Only when nothing kept a reference to those nodes: the
/// parser calls it exactly when it rewinds the arena (`arena_rewindable`), which has that guarantee. A chunk taken
/// since is abandoned (its rows stay untouched address space).
#[inline]
pub fn rewind(cp: Checkpoint) {
    CHUNK.set(cp.0);
}

/// Starts a segment (a file's rows): the thread goes on allocating from its current chunk, and the chunks it takes
/// are recorded until `end_segment`.
pub fn begin_segment() {
    let (begin, begin_end) = CHUNK.get();
    SEGMENT.with_borrow_mut(|seg| {
        debug_assert!(seg.is_none(), "nodetable: nested segment");
        *seg = Some(Segment { begin, begin_end, chunks: Vec::new() });
    });
}

/// Ends the segment: the ranges `start .. end` of rows it kept, in index order. The rows of chunks a rewind abandoned
/// are not in them, and the thread goes on from where the segment's last kept row ends, so the next file's rows
/// share the pages of this file's last one (and nothing is reserved or left as a gap).
pub fn end_segment() -> Vec<(u32, u32)> {
    let seg = SEGMENT.with_borrow_mut(Option::take).expect("nodetable: end_segment without begin_segment");
    let (next, end) = CHUNK.get();
    // The chunk the thread is in now (after any rewind) is the last one of the segment's own rows.
    let mut chunks = Vec::with_capacity(seg.chunks.len() + 1);
    chunks.push((seg.begin, seg.begin_end));
    chunks.extend(seg.chunks);
    let last = chunks.iter().position(|&(_, e)| e == end).expect("nodetable: the current chunk is not one of the segment's");
    let mut ranges = Vec::with_capacity(last + 1);
    for (i, &(start, chunk_end)) in chunks[..=last].iter().enumerate() {
        let stop = if i == last { next } else { chunk_end };
        if stop > start {
            ranges.push((start, stop));
        }
    }
    ranges
}

// ---------------------------------------------------------------------------------------------------------------
// Column access
// ---------------------------------------------------------------------------------------------------------------

#[inline(always)]
#[expect(clippy::inline_always, reason = "every node field read goes through it")]
pub fn kind(idx: u32) -> u16 {
    // SAFETY: `idx` names an allocated row (every `P<Node>` comes from `alloc`).
    unsafe { backing::col::<u16>(KIND, idx).read() }
}

#[inline(always)]
#[expect(clippy::inline_always, reason = "every node field read goes through it")]
pub fn tag(idx: u32) -> u8 {
    // SAFETY: as `kind`.
    unsafe { backing::col::<u8>(TAG, idx).read() }
}

#[inline(always)]
#[expect(clippy::inline_always, reason = "every node field read goes through it")]
pub fn flags(idx: u32) -> u32 {
    // SAFETY: as `kind`.
    unsafe { backing::col::<u32>(FLAGS, idx).read() }
}

#[inline]
pub fn set_flags(idx: u32, flags: u32) {
    // SAFETY: as `kind`; written by the node's owning thread (threading contract).
    unsafe { backing::col::<u32>(FLAGS, idx).write(flags) }
}

/// Address of the row's flags (the shared graph's overlay keys cells by address).
#[inline]
pub fn flags_addr(idx: u32) -> usize {
    backing::col::<u32>(FLAGS, idx).addr()
}

/// The node's id cell (0 = none; assigned once by compare-exchange, read relaxed).
#[inline(always)]
#[expect(clippy::inline_always, reason = "every node id read goes through it")]
pub fn id(idx: u32) -> &'static AtomicU32 {
    // SAFETY: as `kind`; the row lives for the process (or until its file's pages are discarded, after which nothing
    // reads it).
    unsafe { &*backing::col::<AtomicU32>(ID, idx) }
}

#[inline(always)]
#[expect(clippy::inline_always, reason = "every parent read goes through it")]
pub fn parent(idx: u32) -> PKey {
    // SAFETY: as `kind`.
    unsafe { backing::col::<PKey>(PARENT, idx).read() }
}

#[inline]
pub fn set_parent(idx: u32, parent: PKey) {
    // SAFETY: as `set_flags`.
    unsafe { backing::col::<PKey>(PARENT, idx).write(parent) }
}

#[inline]
pub fn parent_addr(idx: u32) -> usize {
    backing::col::<PKey>(PARENT, idx).addr()
}

#[inline(always)]
#[expect(clippy::inline_always, reason = "every data read goes through it")]
pub fn data(idx: u32) -> PKey {
    // SAFETY: as `kind`.
    unsafe { backing::col::<PKey>(DATA, idx).read() }
}

#[inline(always)]
#[expect(clippy::inline_always, reason = "every position read goes through it")]
pub fn loc(idx: u32) -> TextRange {
    // SAFETY: as `kind`.
    unsafe { backing::col::<TextRange>(LOC, idx).read() }
}

#[inline]
pub fn set_loc(idx: u32, loc: TextRange) {
    // SAFETY: as `set_flags`.
    unsafe { backing::col::<TextRange>(LOC, idx).write(loc) }
}

#[inline]
pub fn loc_addr(idx: u32) -> usize {
    backing::col::<TextRange>(LOC, idx).addr()
}

/// Nodes allocated so far (indices handed out, abandoned chunk tails included).
pub fn high_water() -> u32 {
    NEXT.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------------------------------------------
// Giving rows back
// ---------------------------------------------------------------------------------------------------------------

/// Row ranges of freed files not given back yet, batched like the arena's retired regions (each call flushes the
/// TLB of every core running a thread of the process).
static PENDING: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());
static PENDING_ROWS: AtomicU32 = AtomicU32::new(0);
static ANY_DISCARDED: AtomicBool = AtomicBool::new(false);
/// Rows batched before a flush (16 MiB of columns).
const BATCH_ROWS: u32 = ((16 << 20) / BYTES_PER_NODE) as u32;

/// Gives the pages of rows `start .. end` (a freed file's segment, `end_segment`) back to the system, batched.
///
/// # Safety
/// Nothing may read those nodes afterwards.
pub unsafe fn discard(start: u32, end: u32) {
    if end <= start {
        return;
    }
    let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    p.push((start, end));
    // Relaxed: `PENDING_ROWS` is only a batching hint, every read and write of it happens under the `PENDING` lock.
    let rows = PENDING_ROWS.fetch_add(end - start, Ordering::Relaxed) + (end - start);
    if rows >= BATCH_ROWS {
        let ranges = std::mem::take(&mut *p);
        // Relaxed: under the `PENDING` lock, as above.
        PENDING_ROWS.store(0, Ordering::Relaxed);
        drop(p);
        // SAFETY: the caller's promise for every range.
        unsafe { give_back(ranges) };
    }
}

/// Gives back what `discard` has batched.
pub fn flush_discards() {
    let ranges = {
        let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        // Relaxed: under the `PENDING` lock, as in `discard`.
        PENDING_ROWS.store(0, Ordering::Relaxed);
        std::mem::take(&mut *p)
    };
    // SAFETY: every range came from `discard`, whose caller promised the rows are dead.
    unsafe { give_back(ranges) };
}

/// Whether any rows were given back (debug tools must not walk the whole table then).
pub fn any_discarded() -> bool {
    ANY_DISCARDED.load(Ordering::Relaxed)
}

unsafe fn give_back(mut ranges: Vec<(u32, u32)>) {
    if ranges.is_empty() {
        return;
    }
    // Relaxed: a debug-tool hint, read only after the single-threaded discard path has finished.
    ANY_DISCARDED.store(true, Ordering::Relaxed);
    ranges.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    for &(s, e) in &merged {
        // SAFETY: the caller's promise.
        unsafe {
            backing::discard_rows::<u16>(KIND, s, e);
            backing::discard_rows::<u8>(TAG, s, e);
            backing::discard_rows::<u32>(FLAGS, s, e);
            backing::discard_rows::<u32>(ID, s, e);
            backing::discard_rows::<PKey>(PARENT, s, e);
            backing::discard_rows::<PKey>(DATA, s, e);
            backing::discard_rows::<TextRange>(LOC, s, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_written_and_rewound() {
        let loc = TextRange::new(3, 7);
        let a = alloc(5, 2, 0, loc);
        let cp = checkpoint();
        let b = alloc(6, 3, 0, loc);
        assert_eq!(kind(a), 5);
        assert_eq!(tag(b), 3);
        assert_eq!(loc_of(a), loc);
        set_flags(a, 9);
        assert_eq!(flags(a), 9);
        rewind(cp);
        let c = alloc(7, 4, 0, loc);
        assert_eq!(c, b);
        assert_eq!(kind(c), 7);
        assert_eq!(flags(c), 0);
        assert_eq!(key_of(index_of(key_addr(a))), key_of(a));
    }

    fn loc_of(idx: u32) -> TextRange {
        loc(idx)
    }

    fn key_addr(idx: u32) -> usize {
        #[cfg(compressed_ptrs)]
        return crate::reserve::BASE_ADDR + ((idx as usize) << 3);
        #[cfg(not(compressed_ptrs))]
        return key_of(idx);
    }

    #[test]
    fn segment_ranges_are_the_rows_it_kept() {
        let before = alloc(1, 1, 0, TextRange::new(0, 0));
        begin_segment();
        let first = alloc(1, 1, 0, TextRange::new(0, 0));
        let second = alloc(1, 1, 0, TextRange::new(0, 0));
        assert_eq!(second, first + 1);
        let ranges = end_segment();
        assert!(ranges.iter().any(|&(s, e)| s <= first && second < e));
        assert!(ranges.iter().all(|&(s, _)| s > before));
        let after = alloc(1, 1, 0, TextRange::new(0, 0));
        assert_eq!(after, second + 1);
    }
}

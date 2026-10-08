//! The per-thread leak arena behind `P<T>` (see `ptr`): a bump allocator (downward, chunks doubling in size like
//! bumpalo's) with two ways to give memory back that is provably dead:
//!
//! - **Free lists** (`free`, `free_slice`, `P::new_recycled`, `alloc_slice_recycled`): a block whose owner proved
//!   it unreachable goes onto a per-thread list for its size class (8-byte steps up to `MAX_FREE_SIZE`), and the
//!   recycling allocation sites take blocks from there first. Ids of arena objects never depend on addresses, so
//!   reuse changes no output.
//! - **Checkpoints** (`checkpoint`, `rewind`): a speculative parse records the bump position and, when it is rolled
//!   back, moves it back, discarding everything allocated since. This only happens when nothing could have kept a
//!   pointer to that memory: same chunk, and no free or `pin` on this thread in between (`pin` is called by code
//!   that stores freshly allocated data in a structure that outlives the speculation, e.g. a scanner cache).
//!
//! Verification modes (`TSRS_ARENA_POISON=1`, and the census build with `TSRS_CENSUS=1`) never reuse memory: freed
//! blocks and rewound ranges are filled with `POISON` (any later read of a pointer field crashes) or recorded as
//! would-free for the census, which asserts at exit that none of them is reachable.
//!
//! **Regions** (docs/LSP.md "Memory plan for a long-lived server"): a `Region` is an arena of its own that can be
//! freed as a whole. `Region::enter` makes it the current thread's allocation target until the returned scope is
//! dropped (scopes nest); without a scope every allocation goes to the thread's own leak arena, exactly as before.
//! A region owns its chunks, its free lists and a drop list of the values allocated in it whose type needs drop (heap
//! vectors and maps inside links and tables), so freeing it (dropping the last `Region` handle) also releases the
//! heap memory those values own. Freeing is safe only if nothing that outlives the region points into it; the
//! language server ties regions to the objects Go's GC would free together (a parsed file version, a checker, a
//! program), and the census build checks it (a freed region is recorded as would-free, never released).

use std::alloc::Layout;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};

const FIRST_CHUNK: usize = 1 << 20;
const CHUNK_ALIGN: usize = 16;
const PAGE: usize = 4096;
/// Linux with compressed pointers: thread-arena chunks of at least this size (all but a thread's first chunk) are
/// whole 2 MiB blocks backed by transparent huge pages (`reserve::alloc_chunk`), and their sizes are rounded to it,
/// since an unused tail would be resident with the huge page it shares. A thread that allocates less than its first
/// chunk keeps 4 KiB pages. `None` elsewhere.
#[cfg(all(compressed_ptrs, target_os = "linux"))]
const HUGE_THREAD_CHUNK: Option<usize> = Some(crate::reserve::HUGE_CHUNK);
#[cfg(not(all(compressed_ptrs, target_os = "linux")))]
const HUGE_THREAD_CHUNK: Option<usize> = None;
/// Largest chunk a thread arena grows to in compressed mode (larger single allocations still get their own size).
#[cfg(compressed_ptrs)]
const MAX_CHUNK: usize = 64 << 20;
#[cfg(not(compressed_ptrs))]
const MAX_CHUNK: usize = usize::MAX;

/// Largest block size kept on a free list; classes are `size / 8`.
pub const MAX_FREE_SIZE: usize = 512;
const CLASSES: usize = MAX_FREE_SIZE / 8 + 1;

/// Fill byte for freed and rewound memory in poison mode (and in debug builds).
pub const POISON: u8 = 0xA5;

pub struct Arena {
    /// Bump finger in the current chunk (allocation moves it down towards `start`; in a region, up towards `end`).
    ptr: Cell<*mut u8>,
    /// Regions bump upwards: the allocator that hands out a chunk writes its first word (mimalloc's free list), so
    /// the start of every chunk is resident anyway, and the unused part of a region's last chunk (most regions are
    /// small, one per parsed file) should be the untouched end.
    up: bool,
    start: Cell<*mut u8>,
    end: Cell<*mut u8>,
    /// Older chunks: (start, end, finger when the chunk was retired).
    retired: RefCell<Vec<(usize, usize, usize)>>,
    capacity: Cell<usize>,
    /// Bumped by every free and every `pin`; a rewind is skipped when it changed since the checkpoint.
    epoch: Cell<u64>,
    free: [Cell<*mut u8>; CLASSES],
    /// Regions only (`None` for a thread's own arena): the region, for the chunk registry.
    region: Option<Weak<RegionInner>>,
    /// Regions only: values that need drop, dropped when the region is freed.
    drops: RefCell<Vec<DropEntry>>,
    /// Regions only: the slab each chunk was carved from, in chunk order (retired chunks, then the current one).
    slabs: RefCell<Vec<*const Slab>>,
    /// Regions only: the chunks are in the registry (`Region::containing`); scratch regions are not.
    registered: bool,
    /// Regions only: chunks are carved from the thread's large slabs (`Region::new_scratch_in_large_slabs`).
    large_slabs: bool,
    #[cfg(feature = "alloc-profile")]
    pub(crate) free_stats: FreeStats,
}

/// Alloc-profile builds: what this arena's free lists hold, per size class (blocks on the list now and at the class's
/// peak, blocks handed out again, recycling allocations that found the list empty and bumped instead), and the bytes
/// on all lists now and at their peak.
#[cfg(feature = "alloc-profile")]
pub(crate) struct FreeStats {
    pub(crate) blocks: [Cell<u64>; CLASSES],
    pub(crate) peak_blocks: [Cell<u64>; CLASSES],
    pub(crate) reissued: [Cell<u64>; CLASSES],
    pub(crate) misses: [Cell<u64>; CLASSES],
    pub(crate) bytes: Cell<u64>,
    pub(crate) peak_bytes: Cell<u64>,
}

#[cfg(feature = "alloc-profile")]
impl FreeStats {
    const fn new() -> FreeStats {
        FreeStats {
            blocks: [const { Cell::new(0) }; CLASSES],
            peak_blocks: [const { Cell::new(0) }; CLASSES],
            reissued: [const { Cell::new(0) }; CLASSES],
            misses: [const { Cell::new(0) }; CLASSES],
            bytes: Cell::new(0),
            peak_bytes: Cell::new(0),
        }
    }

    fn pushed(&self, class: usize) {
        let n = self.blocks[class].get() + 1;
        self.blocks[class].set(n);
        self.peak_blocks[class].set(self.peak_blocks[class].get().max(n));
        let b = self.bytes.get() + class as u64 * 8;
        self.bytes.set(b);
        self.peak_bytes.set(self.peak_bytes.get().max(b));
    }

    fn popped(&self, class: usize, hit: bool) {
        if hit {
            self.blocks[class].set(self.blocks[class].get() - 1);
            self.bytes.set(self.bytes.get() - class as u64 * 8);
            self.reissued[class].set(self.reissued[class].get() + 1);
        } else {
            self.misses[class].set(self.misses[class].get() + 1);
        }
    }
}

struct DropEntry {
    ptr: *mut u8,
    len: usize,
    drop: unsafe fn(*mut u8, usize),
}

/// # Safety
/// `ptr` holds `len` live `T`s that are never used again (a region's drop list, run once when it is freed).
unsafe fn drop_slice<T>(ptr: *mut u8, len: usize) {
    // SAFETY: this function's contract.
    unsafe { std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(ptr.cast::<T>(), len)) };
}

/// The bump position of the current thread's arena (`checkpoint`).
#[derive(Clone, Copy)]
pub struct Checkpoint {
    ptr: *mut u8,
    start: *mut u8,
    epoch: u64,
    drops: usize,
    #[cfg(feature = "alloc-profile")]
    census_blocks: usize,
}

impl Arena {
    pub(crate) fn new() -> Arena {
        Arena::with_first_chunk(FIRST_CHUNK, None, false, false)
    }

    fn with_first_chunk(first_chunk: usize, region: Option<Weak<RegionInner>>, registered: bool, large_slabs: bool) -> Arena {
        let a = Arena {
            ptr: Cell::new(std::ptr::null_mut()),
            up: region.is_some(),
            start: Cell::new(std::ptr::null_mut()),
            end: Cell::new(std::ptr::null_mut()),
            retired: RefCell::new(Vec::new()),
            capacity: Cell::new(0),
            epoch: Cell::new(0),
            free: [const { Cell::new(std::ptr::null_mut()) }; CLASSES],
            region,
            drops: RefCell::new(Vec::new()),
            slabs: RefCell::new(Vec::new()),
            registered,
            large_slabs,
            #[cfg(feature = "alloc-profile")]
            free_stats: FreeStats::new(),
        };
        a.new_chunk(first_chunk);
        a
    }

    #[inline]
    pub(crate) fn is_region(&self) -> bool {
        self.region.is_some()
    }

    /// Records `len` values of type `T` at `ptr` (just allocated in this arena) for dropping when the region is freed.
    /// Does nothing in a thread's own arena (values there are never dropped).
    #[inline]
    pub(crate) fn track_drop<T>(&self, ptr: *mut T, len: usize) {
        if std::mem::needs_drop::<T>() && self.is_region() && len != 0 {
            self.drops.borrow_mut().push(DropEntry { ptr: ptr.cast(), len, drop: drop_slice::<T> });
        }
    }

    #[inline]
    pub(crate) fn alloc_layout(&self, layout: Layout) -> NonNull<u8> {
        // Profile builds: a zero-sized value (a closure without captures, e.g. a `TypeComparer`) would get the bump
        // position, which is the start of the previous block, and the census would count that word as a reference.
        #[cfg(feature = "alloc-profile")]
        if layout.size() == 0 {
            return NonNull::new(std::ptr::without_provenance_mut(layout.align())).unwrap();
        }
        if self.up {
            return self.alloc_layout_up(layout);
        }
        let ptr = self.ptr.get();
        let new = ptr.addr().wrapping_sub(layout.size()) & !(layout.align() - 1);
        if new >= self.start.get().addr() && new <= ptr.addr() {
            let p = ptr.with_addr(new);
            self.ptr.set(p);
            // SAFETY: `new` lies inside the current chunk (at or above `start`), which is never null.
            return unsafe { NonNull::new_unchecked(p) };
        }
        self.alloc_layout_slow(layout)
    }

    #[inline]
    fn alloc_layout_up(&self, layout: Layout) -> NonNull<u8> {
        let ptr = self.ptr.get();
        let aligned = (ptr.addr() + (layout.align() - 1)) & !(layout.align() - 1);
        let next = aligned.wrapping_add(layout.size());
        if next <= self.end.get().addr() && next >= aligned {
            self.ptr.set(ptr.with_addr(next));
            // SAFETY: `aligned` lies inside the current chunk, which is never null.
            return unsafe { NonNull::new_unchecked(ptr.with_addr(aligned)) };
        }
        self.alloc_layout_slow(layout)
    }

    #[cold]
    #[inline(never)]
    fn alloc_layout_slow(&self, layout: Layout) -> NonNull<u8> {
        let prev = self.end.get().addr() - self.start.get().addr();
        let need = layout.size().checked_add(layout.align()).expect("arena allocation size overflow");
        // A region grows by a quarter of what it has (at least 4 KiB): many regions are small (one per parsed file),
        // and the unused tail of a doubled chunk would dominate their footprint.
        // Compressed pointers: thread-arena chunks stop doubling at `MAX_CHUNK`, since every chunk takes its size out
        // of the fixed reservation, touched or not.
        let next = if self.is_region() {
            (self.capacity.get() / 4).max(PAGE)
        } else if cfg!(compressed_ptrs) {
            (prev * 2).min(MAX_CHUNK)
        } else {
            prev * 2
        };
        self.new_chunk(next.max(need));
        if self.up {
            return self.alloc_layout_up(layout);
        }
        let ptr = self.ptr.get();
        let new = (ptr.addr() - layout.size()) & !(layout.align() - 1);
        debug_assert!(new >= self.start.get().addr());
        let p = ptr.with_addr(new);
        self.ptr.set(p);
        // SAFETY: inside the fresh chunk.
        unsafe { NonNull::new_unchecked(p) }
    }

    fn new_chunk(&self, size: usize) {
        if self.up {
            let size = (size + CENSUS_GAP).next_multiple_of(CHUNK_ALIGN);
            let (base, slab) = slab_carve(size, self.large_slabs);
            self.slabs.borrow_mut().push(slab);
            self.new_chunk_at(base, size);
        } else {
            let huge = HUGE_THREAD_CHUNK.filter(|&h| size >= h);
            let size = size.next_multiple_of(huge.unwrap_or(PAGE));
            let layout = Layout::from_size_align(size, CHUNK_ALIGN).expect("arena chunk layout");
            self.new_chunk_at(os_chunk(layout, huge.is_some()), size);
        }
    }

    fn new_chunk_at(&self, base: *mut u8, size: usize) {
        // Blocks given back by address (`free_block`) are written through this provenance.
        let _ = base.expose_provenance();
        if !self.start.get().is_null() {
            self.retired.borrow_mut().push((self.start.get().addr(), self.end.get().addr(), self.ptr.get().addr()));
        }
        self.start.set(base);
        // SAFETY: one past the end of the allocation.
        self.end.set(unsafe { base.add(size) });
        // (Profile builds: a region's first block does not start at the chunk start, which its arena keeps.)
        self.ptr.set(if self.up { base.wrapping_add(CENSUS_GAP) } else { self.end.get() });
        self.capacity.set(self.capacity.get() + size);
        if let (Some(region), true) = (&self.region, self.registered) {
            REGISTRY.write().unwrap().insert(reg_key(base.addr()), (reg_key(base.addr() + size), Weak::clone(region)));
        }
    }

    /// Region only: gives the unused end of the current chunk back to the slab it was carved from, if it was the
    /// last carve on this thread (a file region right after parse and bind).
    fn trim(&self) {
        let (start, end, ptr) = (self.start.get(), self.end.get(), self.ptr.get());
        let new_end = ptr.addr().next_multiple_of(CHUNK_ALIGN).max(start.addr() + CHUNK_ALIGN);
        let Some(&slab) = self.slabs.borrow().last() else {
            return;
        };
        if new_end >= end.addr() || !slab_trim(slab, end.addr(), new_end) {
            return;
        }
        self.end.set(end.with_addr(new_end));
        self.capacity.set(self.capacity.get() - (end.addr() - new_end));
        if let (Some(region), true) = (&self.region, self.registered) {
            REGISTRY.write().unwrap().insert(reg_key(start.addr()), (reg_key(new_end), Weak::clone(region)));
        }
    }

    /// Every chunk, (start, size).
    fn chunks(&self) -> Vec<(usize, usize)> {
        let mut out = vec![(self.start.get().addr(), self.end.get().addr() - self.start.get().addr())];
        out.extend(self.retired.borrow().iter().map(|&(start, end, _)| (start, end - start)));
        out
    }

    /// Whether `addr` lies in one of this arena's chunks (debug checks).
    #[cfg(debug_assertions)]
    fn owns(&self, addr: usize) -> bool {
        self.chunks().iter().any(|&(start, size)| addr >= start && addr < start + size)
    }

    /// `value` in a block of `layout` (at least `T`'s size and alignment).
    #[inline]
    #[expect(clippy::mut_from_ref, reason = "a bump allocator: each call returns a fresh block nothing else points to (bumpalo's alloc has this signature)")]
    pub(crate) fn alloc_with<T>(&self, layout: Layout, value: T) -> &mut T {
        debug_assert!(layout.size() >= std::mem::size_of::<T>() && layout.align() >= std::mem::align_of::<T>());
        let p = self.alloc_layout(layout).cast::<T>();
        // SAFETY: fresh, aligned, exclusively owned memory for one `T`.
        unsafe {
            p.as_ptr().write(value);
            &mut *p.as_ptr()
        }
    }

    #[inline]
    #[expect(clippy::mut_from_ref, reason = "a fresh block, as in alloc_with")]
    pub(crate) fn alloc_slice_copy<T: Copy>(&self, src: &[T]) -> &mut [T] {
        let p = self.alloc_layout(Layout::for_value(src)).cast::<T>();
        // SAFETY: fresh memory for `src.len()` items; `T: Copy`.
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), p.as_ptr(), src.len());
            std::slice::from_raw_parts_mut(p.as_ptr(), src.len())
        }
    }

    #[inline]
    #[expect(clippy::mut_from_ref, reason = "a fresh block, as in alloc_with")]
    pub(crate) fn alloc_vec<T>(&self, items: Vec<T>) -> &mut [T] {
        let len = items.len();
        let p = self.alloc_layout(Layout::array::<T>(len).expect("arena slice layout")).cast::<T>();
        let mut items = std::mem::ManuallyDrop::new(items);
        // SAFETY: the items move into fresh memory; the vector's buffer is freed without dropping them.
        unsafe {
            std::ptr::copy_nonoverlapping(items.as_ptr(), p.as_ptr(), len);
            items.set_len(0);
            std::mem::ManuallyDrop::drop(&mut items);
            std::slice::from_raw_parts_mut(p.as_ptr(), len)
        }
    }

    #[inline]
    #[expect(clippy::mut_from_ref, reason = "a fresh block, as in alloc_with")]
    pub(crate) fn alloc_str(&self, s: &str) -> &mut str {
        let bytes = self.alloc_slice_copy(s.as_bytes());
        // SAFETY: copied from a `str`.
        unsafe { std::str::from_utf8_unchecked_mut(bytes) }
    }

    /// Total size of the chunks (like bumpalo's `allocated_bytes`).
    pub(crate) fn capacity(&self) -> usize {
        self.capacity.get()
    }

    /// The unused part of the current chunk (below the finger of a thread arena, above it in a region); the
    /// allocation profile reports it per arena.
    #[cfg(feature = "alloc-profile")]
    pub(crate) fn current_chunk_unused(&self) -> usize {
        if self.up {
            self.end.get().addr() - self.ptr.get().addr()
        } else {
            self.ptr.get().addr() - self.start.get().addr()
        }
    }

    /// The used part of every chunk, (start, len).
    pub(crate) fn used_ranges(&self) -> Vec<(usize, usize)> {
        if self.up {
            let mut out = vec![(self.start.get().addr(), self.ptr.get().addr() - self.start.get().addr())];
            out.extend(self.retired.borrow().iter().map(|&(start, _, finger)| (start, finger - start)));
            return out;
        }
        let mut out = vec![(self.ptr.get().addr(), self.end.get().addr() - self.ptr.get().addr())];
        out.extend(self.retired.borrow().iter().map(|&(_, end, finger)| (finger, end - finger)));
        out
    }

    #[inline]
    pub(crate) fn pop_free(&self, class: usize) -> Option<NonNull<u8>> {
        let head = self.free[class].get();
        #[cfg(feature = "alloc-profile")]
        self.free_stats.popped(class, !head.is_null());
        if head.is_null() {
            return None;
        }
        // SAFETY: free blocks hold the next pointer in their first word.
        unsafe {
            #[expect(clippy::cast_ptr_alignment, reason = "free blocks are 8-aligned (push_free's contract)")]
            self.free[class].set(*head.cast_const().cast::<*mut u8>());
            #[cfg(debug_assertions)]
            {
                let size = class * 8;
                let rest = std::slice::from_raw_parts(head.add(8), size - 8);
                assert!(rest.iter().all(|&b| b == POISON), "arena: free block at {head:p} was written after it was freed: {rest:x?}");
            }
            Some(NonNull::new_unchecked(head))
        }
    }

    /// # Safety
    /// `p` points to a dead block of `class * 8` bytes, 8-aligned.
    #[inline]
    pub(crate) unsafe fn push_free(&self, class: usize, p: *mut u8) {
        #[cfg(debug_assertions)]
        // SAFETY: `p` is a dead block of `class * 8` bytes (this function's contract); its first word is kept.
        unsafe { std::ptr::write_bytes(p.add(8), POISON, class * 8 - 8) };
        #[expect(clippy::cast_ptr_alignment, reason = "`p` is 8-aligned (this function's contract)")]
        let next = p.cast::<*mut u8>();
        // SAFETY: the block is dead and at least 8 bytes, so its first word holds the free-list link.
        unsafe { *next = self.free[class].get() };
        self.free[class].set(p);
        #[cfg(feature = "alloc-profile")]
        self.free_stats.pushed(class);
    }

    #[inline]
    pub(crate) fn bump_epoch(&self) {
        self.epoch.set(self.epoch.get() + 1);
    }

    #[inline]
    pub(crate) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            ptr: self.ptr.get(),
            start: self.start.get(),
            epoch: self.epoch.get(),
            drops: self.drops.borrow().len(),
            #[cfg(feature = "alloc-profile")]
            census_blocks: crate::alloc_profile::census_block_count(),
        }
    }

    /// The range allocated since `cp` (lowest address, length) when it can be discarded: same chunk and no free or
    /// pin since.
    #[inline]
    fn rewindable(&self, cp: &Checkpoint) -> Option<(*mut u8, usize)> {
        if cp.start != self.start.get() || cp.epoch != self.epoch.get() {
            return None;
        }
        let now = self.ptr.get();
        if self.up {
            return Some((cp.ptr, now.addr() - cp.ptr.addr()));
        }
        Some((now, cp.ptr.addr() - now.addr()))
    }
}

/// Profile builds map arena chunks far above the heap (from 0x7c00_0000_0000 up). The census decodes every word as a
/// possible pointer; with the arena next to the malloc heap (around 0x200_0000_0000), any pair of 32-bit fields
/// whose second half is 0x200 (a common flag value) "points" into it, which shows up as spurious would-free
/// violations. Bytes 4-5 = 00 7c (NUL then `|`) are rare in numbers and text.
#[cfg(feature = "alloc-profile")]
fn census_chunk(size: usize) -> *mut u8 {
    use std::sync::atomic::AtomicUsize;
    extern "C" {
        fn mmap(addr: *mut std::ffi::c_void, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut std::ffi::c_void;
    }
    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const MAP_PRIVATE: i32 = 2;
    const MAP_ANON: i32 = 0x1000;
    static NEXT: AtomicUsize = AtomicUsize::new(0x7c00_0000_0000);
    let hint = NEXT.fetch_add(size.next_multiple_of(1 << 20) + (1 << 20), Ordering::Relaxed);
    // SAFETY: anonymous private mapping; the hint is only a hint.
    let p = unsafe { mmap(std::ptr::without_provenance_mut(hint), size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0) };
    if p.addr() == usize::MAX {
        return std::ptr::null_mut();
    }
    p.cast()
}

/// A chunk for a thread's arena or a slab. Compressed pointers: from the process-wide reservation (`reserve`), where
/// `huge` asks for transparent huge pages on Linux.
fn os_chunk(layout: Layout, huge: bool) -> *mut u8 {
    #[cfg(compressed_ptrs)]
    return crate::reserve::alloc_chunk(layout.size(), huge);
    #[cfg(not(compressed_ptrs))]
    let _ = huge;
    #[cfg(all(feature = "alloc-profile", not(compressed_ptrs)))]
    let base = census_chunk(layout.size());
    #[cfg(not(any(feature = "alloc-profile", compressed_ptrs)))]
    // SAFETY: non-zero size.
    let base = unsafe { std::alloc::alloc(layout) };
    #[cfg(not(compressed_ptrs))]
    {
        if base.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        base
    }
}

/// Region chunks are carved from per-thread slabs, packed back to back: one region per parsed file means tens of
/// thousands of small regions, and a separate allocation per chunk costs a partly used page at each end (the
/// allocator writes the first word of the next block). A file region's chunk is trimmed to what parse and bind used
/// (`Region::trim`). A slab is released when every chunk carved from it is freed and it is no longer the thread's
/// current slab; large chunks get a slab of their own.
struct Slab {
    base: *mut u8,
    size: usize,
    /// Chunks carved and not yet released, plus one while the slab is a thread's current slab.
    live: AtomicUsize,
    /// A chunk of a retired region (`Region::retire_on_free`) was carved from it: once every chunk is released, its
    /// pages go back to the system but its range is never handed out again (`slab_release`).
    retired: AtomicBool,
}

const SLAB_SIZE: usize = 1 << 20;

/// Slab size of the regions of `Region::new_scratch_in_large_slabs` (the CLI's file regions, most of which are
/// retired once checked). A retired region's pages go back in one system call per run of neighbouring retired pages,
/// and each call flushes the TLB of every core running a thread of the process (`retired`); one thread's file regions
/// in 1 MiB slabs scattered among the other threads' chunks made about 850 calls for vscode's 409 MB of leaves.
/// Untouched pages of a slab cost address space only.
const LARGE_SLAB_SIZE: usize = 16 << 20;

/// Profile builds leave a gap after each carved chunk: a chunk's one-past-the-end pointer (in its arena and in the
/// region registry) would otherwise be the address of the next region's first block, which the census would count
/// as a reference to it.
const CENSUS_GAP: usize = if cfg!(feature = "alloc-profile") { CHUNK_ALIGN } else { 0 };

thread_local! {
    /// The thread's current slab and its bump position (upwards).
    static SLAB: Cell<(*const Slab, usize)> = const { Cell::new((std::ptr::null(), 0)) };
    /// The same for large slabs (`LARGE_SLAB_SIZE`).
    static LARGE_SLAB: Cell<(*const Slab, usize)> = const { Cell::new((std::ptr::null(), 0)) };
}

/// Released standard-size slabs kept for reuse instead of going back to the system (compressed pointers: a
/// decommit and, at the next slab, a commit and fresh page faults). Short-lived regions (one per file during emit)
/// release and take slabs at a high rate. Addresses, at most `SLAB_CACHE_MAX`.
#[cfg(compressed_ptrs)]
static SLAB_CACHE: Mutex<Vec<usize>> = Mutex::new(Vec::new());
#[cfg(compressed_ptrs)]
const SLAB_CACHE_MAX: usize = 64;

fn new_slab(size: usize, live: usize) -> *const Slab {
    let size = size.div_ceil(PAGE) * PAGE;
    #[cfg(compressed_ptrs)]
    if size == SLAB_SIZE {
        let cached = SLAB_CACHE.lock().unwrap().pop();
        if let Some(addr) = cached {
            let base = std::ptr::with_exposed_provenance_mut::<u8>(addr);
            return Box::into_raw(Box::new(Slab { base, size, live: AtomicUsize::new(live), retired: AtomicBool::new(false) }));
        }
    }
    let base = os_chunk(Layout::from_size_align(size, CHUNK_ALIGN).expect("arena slab layout"), false);
    Box::into_raw(Box::new(Slab { base, size, live: AtomicUsize::new(live), retired: AtomicBool::new(false) }))
}

fn slab_carve(size: usize, large: bool) -> (*mut u8, *const Slab) {
    let (current, slab_size) = if large { (&LARGE_SLAB, LARGE_SLAB_SIZE) } else { (&SLAB, SLAB_SIZE) };
    if size > slab_size / 4 {
        let slab = new_slab(size, 1);
        // SAFETY: just made.
        return (unsafe { (*slab).base }, slab);
    }
    let (mut cur, mut bump) = current.with(|s| s.get());
    // SAFETY: the current slab is kept alive by the thread's reference.
    if cur.is_null() || bump + size + CENSUS_GAP > unsafe { (*cur).base.addr() + (*cur).size } {
        if !cur.is_null() {
            slab_release(cur);
        }
        cur = new_slab(slab_size, 1);
        // SAFETY: just made.
        bump = unsafe { (*cur).base.addr() };
    }
    // SAFETY: as above.
    let slab = unsafe { &*cur };
    slab.live.fetch_add(1, Ordering::Relaxed);
    current.with(|s| s.set((cur, bump + size + CENSUS_GAP)));
    (slab.base.with_addr(bump), cur)
}

/// Moves the thread's slab position back from `end` to `new_end` if the chunk ending at `end` was its last carve.
fn slab_trim(slab: *const Slab, end: usize, new_end: usize) -> bool {
    [&SLAB, &LARGE_SLAB].into_iter().any(|current| {
        current.with(|s| {
            let (cur, bump) = s.get();
            if cur == slab && bump == end + CENSUS_GAP {
                s.set((cur, new_end + CENSUS_GAP));
                true
            } else {
                false
            }
        })
    })
}

fn slab_release(slab: *const Slab) {
    slab_release_ex(slab, false);
}

/// `slab_release`; `retire`: the reference is a chunk of a retired region.
fn slab_release_ex(slab: *const Slab, retire: bool) {
    // SAFETY: the caller holds one of the slab's references.
    let s = unsafe { &*slab };
    if retire {
        // Relaxed: the reference count's AcqRel decrement below orders this store before the last release's load.
        s.retired.store(true, Ordering::Relaxed);
    }
    if s.live.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    // Relaxed: every store happened before its thread's decrement, which the AcqRel decrement above acquired.
    if s.retired.load(Ordering::Relaxed) {
        // Its pages go back to the system; its range is never reused (`retired`). Without compressed pointers the
        // slab's memory is kept (it came from the allocator, which would hand the addresses out again).
        #[cfg(all(compressed_ptrs, unix))]
        retired::retire(s.base.addr(), s.size);
        // SAFETY: made by `Box::into_raw` in `new_slab`; this was the last reference.
        drop(unsafe { Box::from_raw(slab.cast_mut()) });
        return;
    }
    // Profile builds map slabs with `mmap` (`census_chunk`) and keep them.
    #[cfg(not(any(feature = "alloc-profile", compressed_ptrs)))]
    // SAFETY: allocated by `new_slab` with this layout; every chunk carved from it was released.
    unsafe {
        std::alloc::dealloc(s.base, Layout::from_size_align(s.size, CHUNK_ALIGN).expect("arena slab layout"))
    };
    #[cfg(compressed_ptrs)]
    {
        let mut cache = SLAB_CACHE.lock().unwrap();
        if s.size == SLAB_SIZE && cache.len() < SLAB_CACHE_MAX {
            cache.push(s.base.expose_provenance());
        } else {
            drop(cache);
            // SAFETY: from `os_chunk` with this size; every chunk carved from it was released.
            unsafe { crate::reserve::release_chunk(s.base, s.size) };
        }
    }
    // SAFETY: made by `Box::into_raw` in `new_slab`; this was the last reference.
    drop(unsafe { Box::from_raw(slab.cast_mut()) });
}

/// Census only: clears `N` bytes of the stack below the caller. The frames that just used the freed or rewound
/// memory are dead but their words stay on the stack, and the padding of the next objects built there copies them
/// into the arena, where the conservative census would count them as references.
#[cfg(feature = "alloc-profile")]
#[inline(never)]
fn scrub_stack<const N: usize>() {
    let mut buf = [0u8; N];
    std::hint::black_box(&mut buf);
}

#[cfg(feature = "alloc-profile")]
pub(crate) fn scrub_stack_for_census() {
    scrub_stack::<{ 256 << 10 }>();
}

/// 0 unknown, 1 normal, 2 poison (`TSRS_ARENA_POISON=1`): freed and rewound memory is filled and never reused.
static MODE: AtomicU8 = AtomicU8::new(0);

#[inline]
pub(crate) fn poison_mode() -> bool {
    match MODE.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("TSRS_ARENA_POISON").is_some_and(|v| v == "1");
            MODE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        m => m == 2,
    }
}

/// Whether freed memory is recorded for the census instead of reused (alloc-profile build, `TSRS_CENSUS=1`).
#[inline]
fn census_mode() -> bool {
    #[cfg(feature = "alloc-profile")]
    {
        crate::alloc_profile::census::recording()
    }
    #[cfg(not(feature = "alloc-profile"))]
    {
        false
    }
}

/// The free-list class of a block of `size` bytes and alignment `align` at address `addr`, or 0 when such blocks
/// are not recycled.
#[inline]
pub const fn free_class(size: usize, align: usize) -> usize {
    if size >= 8 && size <= MAX_FREE_SIZE && size % 8 == 0 && align <= 8 {
        size / 8
    } else {
        0
    }
}

/// Gives a dead block back. Does nothing for sizes that are not recycled.
///
/// # Safety
/// Nothing may use the block afterwards: no live object or local may point into it.
#[inline]
pub(crate) unsafe fn free_block(arena: &Arena, addr: usize, size: usize, align: usize, needs_drop: bool) {
    let class = free_class(size, align);
    if class == 0 || addr % 8 != 0 {
        return;
    }
    // A region's drop list still names the block; it is dropped (not reused) when the region is freed.
    if needs_drop && arena.is_region() {
        return;
    }
    // Free lists belong to the arena that allocated the block: a recycling site frees only what it made, while the
    // same allocation target (thread arena or region) is current.
    #[cfg(debug_assertions)]
    assert!(arena.owns(addr), "arena: block {addr:#x} freed into an arena (or region) that did not allocate it");
    // Write access through the chunk's (exposed) provenance, not through the references the program held.
    let p = std::ptr::with_exposed_provenance_mut::<u8>(addr);
    arena.bump_epoch();
    if census_mode() {
        #[cfg(feature = "alloc-profile")]
        {
            crate::alloc_profile::census_would_free(addr, size);
            scrub_stack::<4096>();
        }
        return;
    }
    if poison_mode() {
        // SAFETY: the block is `size` dead bytes (this function's contract), written through the chunk's provenance.
        unsafe { std::ptr::write_bytes(p, POISON, size) };
        return;
    }
    // SAFETY: a dead block of `class * 8` bytes (`free_class`), 8-aligned (checked above).
    unsafe { arena.push_free(class, p) };
}

/// Rewinds to `cp` if allowed (see the module docs).
#[inline]
pub(crate) fn rewind(arena: &Arena, cp: Checkpoint) {
    let Some((now, len)) = arena.rewindable(&cp) else {
        return;
    };
    if len == 0 {
        return;
    }
    // Values discarded by the rewind are not dropped (as in a thread arena); forget their drop entries.
    arena.drops.borrow_mut().truncate(cp.drops);
    if census_mode() {
        #[cfg(feature = "alloc-profile")]
        {
            crate::alloc_profile::census_would_free_since(cp.census_blocks);
            scrub_stack::<16384>();
        }
        return;
    }
    if poison_mode() {
        // SAFETY: `now .. now + len` is the range allocated since the checkpoint, in the current chunk.
        unsafe { std::ptr::write_bytes(now, POISON, len) };
        return;
    }
    #[cfg(debug_assertions)]
    // SAFETY: as above.
    unsafe {
        std::ptr::write_bytes(now, POISON, len)
    };
    #[cfg(feature = "alloc-profile")]
    assert!(!crate::alloc_profile::census::active(), "arena rewind while the census records");
    arena.ptr.set(cp.ptr);
}


// ---------------------------------------------------------------------------------------------------------------
// Regions and the allocation target
// ---------------------------------------------------------------------------------------------------------------

/// Registry keys and ends are addresses + 1 (order is kept): the map's vacated slots keep old values, which must not
/// look like references to blocks in the census build.
#[inline]
fn reg_key(addr: usize) -> usize {
    addr + 1
}

/// Chunk start -> (chunk end, region), for `Region::containing` (both as `reg_key`).
static REGISTRY: RwLock<BTreeMap<usize, (usize, Weak<RegionInner>)>> = RwLock::new(BTreeMap::new());
/// Set once the first region exists; until then (the CLI) owner lookups return immediately.
static ANY_REGION: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The arena allocations go to: null until the thread first allocates (then its own arena), or a region.
    pub(crate) static CURRENT: Cell<*const Arena> = const { Cell::new(std::ptr::null()) };
    /// Entered scopes, innermost last: (token, arena). Empty: the thread's own arena is current.
    static SCOPES: RefCell<Vec<(u64, *const Arena)>> = const { RefCell::new(Vec::new()) };
    /// Scope tokens: a scope is `!Send`, so tokens only need to be unique on their thread (a global counter was a
    /// contended cache line once scratch regions push a scope per checker call).
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
    /// While a scratch region is entered on this thread (`Region::enter_scratch`): its arena, and the allocation
    /// target that was current when it was entered (`escape_scratch`). Null when none is.
    static SCRATCH: Cell<(*const Arena, *const Arena)> = const { Cell::new((std::ptr::null(), std::ptr::null())) };
}

/// A lock that the thread holding it may take again (a region entered while it is already entered).
struct OwnerLock {
    /// (owner, depth, threads waiting).
    state: Mutex<(Option<std::thread::ThreadId>, u32, u32)>,
    released: Condvar,
}

impl OwnerLock {
    fn lock(&self) {
        let me = std::thread::current().id();
        let mut st = self.state.lock().unwrap();
        loop {
            match st.0 {
                None => {
                    st.0 = Some(me);
                    st.1 = 1;
                    return;
                }
                Some(owner) if owner == me => {
                    st.1 += 1;
                    return;
                }
                Some(_) => {
                    st.2 += 1;
                    st = self.released.wait(st).unwrap();
                    st.2 -= 1;
                }
            }
        }
    }

    fn unlock(&self) {
        let mut st = self.state.lock().unwrap();
        st.1 -= 1;
        if st.1 == 0 {
            st.0 = None;
            // A notification is a system call even when nobody waits (std's futex condvar); a file region is entered
            // and left about four times by one thread (parse, bind, two trims) and nearly never waited for.
            let waiting = st.2 != 0;
            drop(st);
            if waiting {
                self.released.notify_one();
            }
        }
    }
}

pub(crate) struct RegionInner {
    arena: Box<Arena>,
    /// Held by the thread that has the region entered: a region's arena is used by one thread at a time.
    lock: OwnerLock,
    /// Addresses of objects outside the region registered as owned by it (`adopt_owner`).
    owners: Mutex<Vec<usize>>,
    /// Run when the region is freed, before anything else (`on_free`).
    on_free: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    /// `retire_on_free`.
    retire: AtomicBool,
}

#[expect(clippy::non_send_fields_in_send_ty, reason = "the arena's cells: see the SAFETY comment")]
// SAFETY: the arena's cells are only touched by the thread holding `lock` (or, before the region is shared, by its
// creator), and by `Drop`, when no handle exists any more.
unsafe impl Send for RegionInner {}
// SAFETY: as for `Send`: shared handles reach the arena's cells only while holding `lock`.
unsafe impl Sync for RegionInner {}

/// A freeable arena (see the module docs). Cloning shares it; it is freed when the last handle is dropped.
#[derive(Clone)]
pub struct Region(Arc<RegionInner>);

impl Region {
    /// A new, empty region whose first chunk has at least `first_chunk` bytes (rounded up to a page).
    pub fn new(first_chunk: usize) -> Region {
        ANY_REGION.store(true, Ordering::Relaxed);
        Region::new_in(first_chunk, true, false)
    }

    /// A region to be used only through `enter_scratch` (one file's emit): like `new`, but its chunks are not in the
    /// registry, so `containing` never returns it and creating or freeing it takes no global lock. Lazily filled data
    /// of an object inside it (`enter_owner`) goes to the current target, the region itself while it is entered. A
    /// process with only scratch regions (the CLI) keeps `enter_owner` / `enter_table_owner` free.
    pub fn new_scratch(first_chunk: usize) -> Region {
        Region::new_in(first_chunk, false, false)
    }

    /// `new_scratch` for one of many regions that are mostly retired (`retire_on_free`) in an order unrelated to
    /// their creation (the CLI's file regions): its chunks are carved from the thread's large slabs
    /// (`LARGE_SLAB_SIZE`), so the regions one thread made lie together and retired neighbours give their pages back
    /// in long runs, one system call (and one TLB flush on every core of the process) each.
    pub fn new_scratch_in_large_slabs(first_chunk: usize) -> Region {
        Region::new_in(first_chunk, false, true)
    }

    fn new_in(first_chunk: usize, registered: bool, large_slabs: bool) -> Region {
        Region(Arc::new_cyclic(|weak| RegionInner {
            arena: Box::new(Arena::with_first_chunk(first_chunk.max(PAGE), Some(Weak::clone(weak)), registered, large_slabs)),
            lock: OwnerLock { state: Mutex::new((None, 0, 0)), released: Condvar::new() },
            owners: Mutex::new(Vec::new()),
            on_free: Mutex::new(Vec::new()),
            retire: AtomicBool::new(false),
        }))
    }

    /// When the region is freed, its pages go back to the system and its address range is never handed out again
    /// (`reserve::discard`), instead of its slabs being reused: for a region freed while tables keyed by the address
    /// of an object in it may keep stale entries (a checked file's tree, `tsrs_compiler` fileregions.rs). The pages a
    /// chunk shares with a neighbouring chunk of the slab stay until the whole slab is released.
    pub fn retire_on_free(&self) {
        // Relaxed: read by `Drop`, after the last handle's release, which the `Arc` orders after this store.
        self.0.retire.store(true, Ordering::Relaxed);
    }

    /// Makes this region the current thread's allocation target until the scope is dropped. Waits while another
    /// thread has it entered.
    pub fn enter(&self) -> RegionScope {
        self.0.lock.lock();
        RegionScope::push(&raw const *self.0.arena, Some(self.clone()))
    }

    /// The region one of whose chunks contains `addr`.
    pub fn containing(addr: usize) -> Option<Region> {
        if !ANY_REGION.load(Ordering::Relaxed) {
            return None;
        }
        let reg = REGISTRY.read().unwrap();
        let (_, (end, region)) = reg.range(..=reg_key(addr)).next_back()?;
        if reg_key(addr) >= *end {
            return None;
        }
        region.upgrade().map(Region)
    }

    /// `containing` for many addresses under one registry lock.
    pub fn containing_all(addrs: impl Iterator<Item = usize>) -> Vec<Option<Region>> {
        if !ANY_REGION.load(Ordering::Relaxed) {
            return addrs.map(|_| None).collect();
        }
        let reg = REGISTRY.read().unwrap();
        addrs
            .map(|addr| {
                let (_, (end, region)) = reg.range(..=reg_key(addr)).next_back()?;
                if reg_key(addr) >= *end {
                    return None;
                }
                region.upgrade().map(Region)
            })
            .collect()
    }

    /// Runs `f` when the region is freed (also in the census and poison modes, which keep the memory): for tables
    /// outside the region that must forget pointers into it.
    pub fn on_free(&self, f: Box<dyn FnOnce() + Send>) {
        self.0.on_free.lock().unwrap().push(f);
    }

    /// Registers the object at `addr` (not in any region, e.g. a heap object shared by several region owners) as owned
    /// by this region, so `enter_owner(addr)` routes to it while the region lives.
    pub fn adopt_owner(&self, addr: usize) {
        self.0.owners.lock().unwrap().push(addr);
        REGISTRY.write().unwrap().insert(reg_key(addr), (reg_key(addr + 1), Arc::downgrade(&self.0)));
    }

    /// Total size of the region's chunks.
    pub fn allocated_bytes(&self) -> usize {
        self.0.arena.capacity()
    }

    /// Gives the unused end of the region's current chunk back (call on the thread that last allocated in it, right
    /// after a phase that will not allocate much more, such as parsing and binding a file).
    pub fn trim(&self) {
        let _scope = self.enter();
        self.0.arena.trim();
    }

    /// Bytes in use in the region's chunks (call while no other thread has the region entered).
    pub fn used_bytes(&self) -> usize {
        self.0.arena.used_ranges().iter().map(|&(_, len)| len).sum()
    }

    /// Number of values waiting to be dropped when the region is freed (diagnostics).
    pub fn drop_entries(&self) -> usize {
        self.0.arena.drops.borrow().len()
    }

    /// Forgets the values waiting to be dropped (and frees the list): for a region that will never be freed, whose
    /// values then live for the rest of the process like those of a thread arena. Values allocated afterwards are
    /// tracked again.
    pub fn forget_drops(&self) {
        let _scope = self.enter();
        *self.0.arena.drops.borrow_mut() = Vec::new();
    }

    pub fn ptr_eq(&self, other: &Region) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Region({:p})", Arc::as_ptr(&self.0))
    }
}

/// While alive, allocations on this thread go to the scope's arena (a region, or the thread's own arena for
/// `enter_thread_arena`). Scopes nest; dropping one restores the target that was current before it.
pub struct RegionScope {
    token: u64,
    region: Option<Region>,
    // Thread-bound: the scope edits the thread's target stack.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl RegionScope {
    fn push(arena: *const Arena, region: Option<Region>) -> RegionScope {
        let token = NEXT_TOKEN.with(|t| {
            let token = t.get();
            t.set(token + 1);
            token
        });
        SCOPES.with(|s| s.borrow_mut().push((token, arena)));
        CURRENT.with(|c| c.set(arena));
        RegionScope { token, region, _not_send: std::marker::PhantomData }
    }
}

impl Drop for RegionScope {
    fn drop(&mut self) {
        SCOPES.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(i) = s.iter().rposition(|&(t, _)| t == self.token) {
                s.remove(i);
            }
            let top = s.last().map_or(std::ptr::null(), |&(_, a)| a);
            CURRENT.with(|c| c.set(top));
        });
        if let Some(region) = &self.region {
            region.0.lock.unlock();
        }
    }
}

/// The region that is the current thread's allocation target, if any (`None`: the thread's own arena).
pub fn current_region() -> Option<Region> {
    let p = CURRENT.with(|c| c.get());
    if p.is_null() {
        return None;
    }
    // SAFETY: a non-null `CURRENT` is the thread arena or the arena of a region kept alive by an entered scope.
    let arena = unsafe { &*p };
    arena.region.as_ref().and_then(|w| w.upgrade()).map(Region)
}

/// Makes the current thread's own (never freed) arena the allocation target until the scope is dropped: for data
/// that outlives any region, such as process-wide lazily initialized statics.
pub fn enter_thread_arena() -> RegionScope {
    let own = std::ptr::from_ref::<Arena>(crate::ptr::own_arena());
    RegionScope::push(own, None)
}

/// Routes allocations to the region that owns the object at `addr` (lazily initialized data of a shared object, such
/// as a source file's JSDoc cache, must live and die with that object, not with the checker that happens to fill
/// it). An object outside every region gets the thread's own arena. No scope (and no cost beyond one load) while no
/// region exists, as in the CLI.
#[inline]
pub fn enter_owner(addr: usize) -> Option<RegionScope> {
    if !ANY_REGION.load(Ordering::Relaxed) {
        // Scratch regions are not registered: data of an object outside the scratch region must not land in it.
        return escape_scratch_unless_inside(addr);
    }
    Some(match Region::containing(addr) {
        Some(region) => region.enter(),
        None => enter_thread_arena(),
    })
}

/// Routes allocations for a value stored in a shared table that may outlive the current allocation target (a cache
/// that later program versions copy, filled by whichever thread happens to look something up): they stay in the
/// current target only if the table at `addr` lives in it, and go to the thread's own (never freed) arena otherwise.
/// Unlike `enter_owner`, never waits for a region another thread has entered. No scope (and no cost beyond one load)
/// while no region exists, as in the CLI.
#[inline]
pub fn enter_table_owner(addr: usize) -> Option<RegionScope> {
    if !ANY_REGION.load(Ordering::Relaxed) {
        return escape_scratch_unless_inside(addr);
    }
    let current = CURRENT.with(|c| c.get());
    match Region::containing(addr) {
        Some(region) if std::ptr::eq(&raw const *region.0.arena, current) => None,
        _ => Some(enter_thread_arena()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Scratch regions (per-file emit, notes/mem-emit-regions.md)
// ---------------------------------------------------------------------------------------------------------------

/// While alive, the region it was made from is the thread's allocation target as with `Region::enter`, and also the
/// thread's **scratch region**: code that knows its allocations die with the scratch region allocates there even
/// while escaped (`P::new_scratch`, `alloc_str_scratch`, ...), and code whose allocations must outlive it escapes
/// to the target that was current before it (`escape_scratch`). Emit uses one per file: the transformers, the
/// printer and the node builder's per-call state allocate in it, the checker escapes (its types, symbols and caches
/// outlive the file).
pub struct ScratchScope {
    saved: (*const Arena, *const Arena),
    _scope: RegionScope,
}

impl Region {
    /// `enter`, and makes the region the thread's scratch region (see `ScratchScope`). Scratch scopes nest; the
    /// innermost one is the scratch region.
    pub fn enter_scratch(&self) -> ScratchScope {
        let outer = CURRENT.with(|c| c.get());
        let outer = if outer.is_null() { std::ptr::from_ref::<Arena>(crate::ptr::own_arena()) } else { outer };
        let scope = self.enter();
        let saved = SCRATCH.with(|s| s.replace((&raw const *self.0.arena, outer)));
        ScratchScope { saved, _scope: scope }
    }
}

impl Drop for ScratchScope {
    fn drop(&mut self) {
        SCRATCH.with(|s| s.set(self.saved));
    }
}

/// Inside a scratch region, makes the allocation target that was current when it was entered (the thread's own arena,
/// or an enclosing region) the target until the scope is dropped: for code whose allocations may outlive the scratch
/// region (the checker called from emit, program caches, diagnostics). `None` (no cost beyond a thread-local read)
/// outside scratch regions or when that target is already current.
#[inline]
pub fn escape_scratch() -> Option<RegionScope> {
    let (scratch, outer) = SCRATCH.with(|s| s.get());
    if scratch.is_null() || CURRENT.with(|c| c.get()) == outer {
        return None;
    }
    Some(RegionScope::push(outer, None))
}

/// `escape_scratch` unless the object at `addr` lives in the scratch region (`enter_owner` without registered regions).
#[inline]
fn escape_scratch_unless_inside(addr: usize) -> Option<RegionScope> {
    if scratch_arena().is_null() || scratch_contains(addr) {
        return None;
    }
    escape_scratch()
}

/// Whether a scratch region is entered on this thread.
#[inline]
pub fn scratch_active() -> bool {
    !SCRATCH.with(|s| s.get()).0.is_null()
}

/// Whether `addr` lies in the thread's scratch region (for caches outside it that must forget keys in it).
pub fn scratch_contains(addr: usize) -> bool {
    let s = scratch_arena();
    if s.is_null() {
        return false;
    }
    // SAFETY: the scratch region is kept alive by its entered scope on this thread, which alone uses its arena.
    let a = unsafe { &*s };
    (a.start.get().addr() <= addr && addr < a.end.get().addr()) || a.retired.borrow().iter().any(|&(start, end, _)| start <= addr && addr < end)
}

/// The thread's scratch region's arena, if one is entered.
#[inline]
pub(crate) fn scratch_arena() -> *const Arena {
    SCRATCH.with(|s| s.get()).0
}

impl Drop for RegionInner {
    fn drop(&mut self) {
        for f in self.on_free.get_mut().unwrap().drain(..) {
            f();
        }
        let arena = &*self.arena;
        let chunks = arena.chunks();
        if arena.registered || !self.owners.get_mut().unwrap().is_empty() {
            let mut reg = REGISTRY.write().unwrap();
            for &(start, _) in &chunks {
                reg.remove(&reg_key(start));
            }
            for addr in self.owners.get_mut().unwrap().drain(..) {
                reg.remove(&reg_key(addr));
            }
        }
        let drops = std::mem::take(&mut *arena.drops.borrow_mut());
        for d in drops {
            // SAFETY: `len` values of the entry's type were allocated at `ptr` in this region and never dropped (a
            // freed block of a type that needs drop is not reused, a rewound one has no entry).
            unsafe { (d.drop)(d.ptr, d.len) };
        }
        if census_mode() {
            // The heap memory the values owned is released (above), the region's own memory is recorded as
            // would-free and kept, never reused: the census checks at exit that nothing reachable points into it.
            #[cfg(feature = "alloc-profile")]
            for (start, len) in arena.used_ranges() {
                crate::alloc_profile::census_would_free_range(start, len);
            }
            return;
        }
        if poison_mode() {
            // Kept mapped and filled, so any later use through a stale pointer crashes.
            for (start, len) in arena.used_ranges() {
                // SAFETY: the used part of a chunk of this region; nothing may use it any more.
                unsafe { std::ptr::write_bytes(std::ptr::with_exposed_provenance_mut::<u8>(start), POISON, len) };
            }
            return;
        }
        // Relaxed: see `retire_on_free`.
        let retire = self.retire.load(Ordering::Relaxed);
        #[cfg(all(compressed_ptrs, unix))]
        if retire {
            for &(start, size) in &chunks {
                retired::retire(start, size);
            }
        }
        drop(chunks);
        for &slab in arena.slabs.borrow().iter() {
            slab_release_ex(slab, retire);
        }
    }
}

/// Gives back the pages of retired regions (`Region::retire_on_free`) that nothing uses any more, without ever giving
/// their ranges back (`reserve::discard`).
///
/// Retired ranges (the chunks of retired regions, and retired slabs once their last chunk goes) are kept as coalesced
/// spans, so the page that two neighbouring chunks share goes once both are retired: file regions are carved back to
/// back, leaves are often neighbours (vscode: 4,691 leaf chunks form 924 runs), and with 16 KiB pages the shared pages
/// are a tenth of what the leaves hold. They are given back in batches of `BATCH` bytes (`flush_retired` gives back the
/// rest): each `madvise` / `mmap` call flushes the TLB of every core running a thread of the process, and one call per
/// chunk cost the check pass 2-3% at 32 checkers on Linux, against one per run of neighbouring pages in a batch.
#[cfg(all(compressed_ptrs, unix))]
mod retired {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Bytes retired before a batch is given back.
    const BATCH: usize = 16 << 20;

    struct State {
        /// Retired spans, start -> end, coalesced (touching or overlapping ranges merge).
        spans: BTreeMap<usize, usize>,
        /// Retired since the last batch, not given back yet.
        pending: Vec<(usize, usize)>,
        pending_bytes: usize,
    }

    static STATE: Mutex<State> = Mutex::new(State { spans: BTreeMap::new(), pending: Vec::new(), pending_bytes: 0 });
    /// `stats`: calls that gave pages back, and their bytes.
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static BYTES: AtomicUsize = AtomicUsize::new(0);

    /// Retires `start .. start + len` (nothing uses it any more, and it is never handed out again).
    pub(super) fn retire(start: usize, len: usize) {
        let (pages, calls) = {
            let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
            s.pending.push((start, start + len));
            s.pending_bytes += len;
            if s.pending_bytes < BATCH {
                return;
            }
            let pages = take_pages(&mut s);
            let calls = spans_of(&s, &pages);
            (pages, calls)
        };
        give_back(&pages, &calls);
    }

    pub(super) fn flush() {
        let (pages, calls) = {
            let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
            let pages = take_pages(&mut s);
            let calls = spans_of(&s, &pages);
            (pages, calls)
        };
        give_back(&pages, &calls);
    }

    /// The whole pages of each span that `pages` (from `take_pages`) lie in: a span's pages are all retired, and the
    /// ones it absorbed were given back before (they hold no memory, and giving them back again costs nothing), so
    /// one call per span gives back what `pages` lists. Freed with many checkers, a batch's ranges sit between ranges
    /// retired earlier, and their pages would take a call for each piece; each call flushes the TLB of every core
    /// running a thread of the process.
    fn spans_of(s: &State, pages: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let page = crate::reserve::page_size();
        let mut out: Vec<(usize, usize)> = Vec::with_capacity(pages.len());
        for &(first, _) in pages {
            let Some((&lo, &hi)) = s.spans.range(..=first).next_back() else { continue };
            let whole = (lo.next_multiple_of(page), hi & !(page - 1));
            if out.last() != Some(&whole) {
                out.push(whole);
            }
        }
        out
    }

    pub(super) fn stats() -> (usize, usize) {
        // Relaxed: counters, read after the threads that retire joined.
        (CALLS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed))
    }

    /// Merges the pending ranges into the spans and returns the pages that became wholly retired: for each pending
    /// range, the whole pages of its coalesced span minus those of the spans it absorbed (given back already, or
    /// earlier in this batch), merged where they touch.
    fn take_pages(s: &mut State) -> Vec<(usize, usize)> {
        let page = crate::reserve::page_size();
        let interior = |lo: usize, hi: usize| (lo.next_multiple_of(page), hi & !(page - 1));
        let mut pages = Vec::new();
        for (start, end) in std::mem::take(&mut s.pending) {
            let (mut lo, mut hi) = (start, end);
            let mut absorbed: Vec<(usize, usize)> = Vec::new();
            if let Some((&ps, &pe)) = s.spans.range(..=start).next_back() {
                if pe >= start {
                    absorbed.push((ps, pe));
                    lo = ps;
                    hi = hi.max(pe);
                    s.spans.remove(&ps);
                }
            }
            while let Some((&ns, &ne)) = s.spans.range(lo..).next() {
                if ns > hi {
                    break;
                }
                absorbed.push((ns, ne));
                hi = hi.max(ne);
                s.spans.remove(&ns);
            }
            s.spans.insert(lo, hi);
            // The span's whole pages, minus the absorbed spans' (sorted, disjoint, inside it).
            let (mut next, last) = interior(lo, hi);
            for (a, b) in absorbed {
                let (done_first, done_last) = interior(a, b);
                if done_last <= done_first {
                    continue;
                }
                if done_first > next {
                    pages.push((next, done_first.min(last)));
                }
                next = next.max(done_last);
            }
            if last > next {
                pages.push((next, last));
            }
        }
        pages.retain(|&(first, last)| last > first);
        s.pending_bytes = 0;
        pages.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(pages.len());
        for (first, last) in pages {
            match merged.last_mut() {
                Some(prev) if prev.1 >= first => prev.1 = prev.1.max(last),
                _ => merged.push((first, last)),
            }
        }
        merged
    }

    #[cfg(test)]
    mod tests {
        use super::{spans_of, take_pages, State};
        use std::collections::BTreeMap;

        fn batch(s: &mut State, ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
            for &(start, end) in ranges {
                s.pending.push((start, end));
            }
            take_pages(s)
        }

        /// A page two neighbouring retired chunks share goes once both are retired; pages given back are not given
        /// back again when a slab around them is retired.
        #[test]
        fn neighbours_share_their_boundary_page() {
            let p = crate::reserve::page_size();
            let mut s = State { spans: BTreeMap::new(), pending: Vec::new(), pending_bytes: 0 };
            let a = (10 * p + 100, 12 * p + 50);
            let b = (12 * p + 50, 14 * p);
            assert_eq!(batch(&mut s, &[a]), [(11 * p, 12 * p)]);
            // The page at 12p (part a, part b) goes with b.
            assert_eq!(batch(&mut s, &[b]), [(12 * p, 14 * p)]);
            // Their slab: only what is outside them.
            assert_eq!(batch(&mut s, &[(8 * p, 16 * p)]), [(8 * p, 11 * p), (14 * p, 16 * p)]);
            // In one batch, in any order: the same pages, merged.
            let mut t = State { spans: BTreeMap::new(), pending: Vec::new(), pending_bytes: 0 };
            assert_eq!(batch(&mut t, &[b, a]), [(11 * p, 14 * p)]);
            // A chunk not touching them: its own whole pages only.
            assert_eq!(batch(&mut t, &[(20 * p + 1, 23 * p - 1)]), [(21 * p, 22 * p)]);
        }

        /// Ranges retired on both sides of a span given back earlier go back in one call over the whole span.
        #[test]
        fn a_batch_gives_back_whole_spans() {
            let p = crate::reserve::page_size();
            let mut s = State { spans: BTreeMap::new(), pending: Vec::new(), pending_bytes: 0 };
            assert_eq!(batch(&mut s, &[(10 * p, 12 * p)]), [(10 * p, 12 * p)]);
            let pages = batch(&mut s, &[(8 * p, 10 * p), (12 * p, 14 * p), (30 * p, 31 * p)]);
            assert_eq!(pages, [(8 * p, 10 * p), (12 * p, 14 * p), (30 * p, 31 * p)]);
            assert_eq!(spans_of(&s, &pages), [(8 * p, 14 * p), (30 * p, 31 * p)]);
        }
    }

    /// Gives back `calls` (whole pages of retired spans, `spans_of`); `pages`, the ones not given back before, are
    /// what `stats` counts.
    fn give_back(pages: &[(usize, usize)], calls: &[(usize, usize)]) {
        for &(first, last) in calls {
            // SAFETY: whole pages inside retired spans: chunks and slabs nothing uses any more, never handed out
            // again; `first` is page aligned (and so is `last - first`).
            unsafe { crate::reserve::discard(std::ptr::with_exposed_provenance_mut::<u8>(first), last - first) };
        }
        // Relaxed: counters (see `stats`).
        CALLS.fetch_add(calls.len(), Ordering::Relaxed);
        BYTES.fetch_add(pages.iter().map(|&(first, last)| last - first).sum(), Ordering::Relaxed);
    }
}

/// Gives back the pages of the regions retired since the last batch (`Region::retire_on_free`), for a caller that has
/// freed the last of them (the end of the type-check pass).
pub fn flush_retired() {
    #[cfg(all(compressed_ptrs, unix))]
    retired::flush();
}

/// (calls that gave pages of retired regions back to the system, bytes given back), for `TSRS_FREE_LEAVES=stats`.
pub fn retired_stats() -> (usize, usize) {
    #[cfg(all(compressed_ptrs, unix))]
    return retired::stats();
    #[cfg(not(all(compressed_ptrs, unix)))]
    (0, 0)
}

#[cfg(test)]
mod tests {
    use crate::{alloc_slice_recycled, arena_checkpoint, arena_pin, arena_rewind, free, free_slice, P};

    #[test]
    fn free_list_reuses_blocks_of_the_same_size() {
        let a = P::new([1u64, 2]);
        let addr = a.addr();
        // SAFETY: `a` is not used again.
        unsafe { free!(a) };
        let b = P::new_recycled([3u64, 4]);
        assert_eq!(b.addr(), addr);
        assert_eq!(*b, [3, 4]);
        let s = alloc_slice_recycled(&[5u64, 6]);
        let s_addr = s.as_ptr() as usize;
        assert_ne!(s_addr, addr);
        // SAFETY: `s` is not used again (not even for its address: Miri).
        unsafe { free_slice!(s) };
        let t = alloc_slice_recycled(&[7u64, 8]);
        assert_eq!(t.as_ptr() as usize, s_addr);
        assert_eq!(t, &[7, 8]);
    }

    /// A callee that frees a slice its caller made takes it as `*const [T]` (`free_slice_ptr`): a `&[T]` parameter
    /// is a protected borrow for the whole call, and the free-list link written into the block would be undefined
    /// behaviour (Miri reports it; LLVM deleted that write once, notes/fix-arena-recycle-uaf.md). Run under Miri:
    /// `cargo +nightly miri test -p tsrs_core --features plain-ptrs --lib arena::tests`.
    #[test]
    fn a_callee_frees_a_slice_passed_as_a_raw_pointer() {
        /// # Safety
        /// `s` is a whole arena slice that nothing uses afterwards.
        unsafe fn recycle_list(s: *const [u64]) {
            // SAFETY: this function's contract.
            unsafe { crate::free_slice_ptr(s) };
        }
        let s = alloc_slice_recycled(&[1u64, 2, 3]);
        let s_addr = s.as_ptr() as usize;
        // SAFETY: `s` is not used again.
        unsafe { recycle_list(s) };
        let t = alloc_slice_recycled(&[4u64, 5, 6]);
        assert_eq!(t.as_ptr() as usize, s_addr);
        assert_eq!(t, &[4, 5, 6]);
        // The link was written: the list is empty again, so the next block of that size is fresh.
        let u = alloc_slice_recycled(&[7u64, 8, 9]);
        assert_ne!(u.as_ptr() as usize, s_addr);
        assert_eq!((t, u), (&[4u64, 5, 6][..], &[7u64, 8, 9][..]));
    }

    #[test]
    #[ignore = "a negative control for Miri: it must report undefined behaviour (a write through a protected borrow)"]
    fn miri_rejects_freeing_a_reference_parameter() {
        fn recycle_list(s: &'static [u64]) {
            // SAFETY: deliberately wrong: `s` is a protected borrow of the block for this whole call.
            unsafe { free_slice!(s) }; // source.py: deliberate
        }
        recycle_list(alloc_slice_recycled(&[1u64, 2, 3]));
    }

    #[test]
    fn rewind_discards_speculative_allocations_unless_pinned() {
        let before = P::new(1u64);
        let cp = arena_checkpoint();
        let spec = P::new(2u64).addr();
        arena_rewind(cp);
        let after = P::new(3u64);
        assert_eq!(after.addr(), spec);
        assert_eq!((*before, *after), (1, 3));

        let cp = arena_checkpoint();
        let kept = P::new(4u64);
        arena_pin();
        arena_rewind(cp);
        let next = P::new(5u64);
        assert_ne!(next.addr(), kept.addr());
        assert_eq!(*kept, 4);
    }

    #[test]
    fn region_scopes_route_allocations_and_free_drops_values() {
        use super::{enter_owner, enter_thread_arena, Region};
        use std::rc::Rc;
        let outside = P::new(1u64);
        let counter = Rc::new(());
        let region = Region::new(4096);
        let (inside, nested_outside, inner) = {
            let _scope = region.enter();
            let inside = P::new(Rc::clone(&counter));
            let v = crate::alloc_vec(vec![Rc::clone(&counter), Rc::clone(&counter)]);
            assert_eq!(v.len(), 2);
            let nested_outside = {
                let _global = enter_thread_arena();
                P::new(2u64)
            };
            let inner = Region::new(4096);
            {
                let _s = inner.enter();
                let x = P::new(3u64);
                assert!(Region::containing(x.addr()).unwrap().ptr_eq(&inner));
                // Owner routing back into the outer region from inside another one.
                let _o = enter_owner(inside.addr());
                let y = P::new(4u64);
                assert!(Region::containing(y.addr()).unwrap().ptr_eq(&region));
            }
            (inside.addr(), nested_outside, P::new(5u64))
        };
        assert!(Region::containing(outside.addr()).is_none());
        assert!(Region::containing(nested_outside.addr()).is_none());
        assert!(Region::containing(inner.addr()).unwrap().ptr_eq(&region));
        assert!(Region::containing(inside).unwrap().ptr_eq(&region));
        assert_eq!(Rc::strong_count(&counter), 4);
        drop(region);
        assert!(Region::containing(inside).is_none());
        assert_eq!(Rc::strong_count(&counter), 1);
        // Back on the thread arena.
        assert!(Region::containing(P::new(6u64).addr()).is_none());
        assert_eq!((*outside, *nested_outside), (1, 2));
    }

    #[test]
    fn region_free_lists_and_rewinds_stay_in_the_region() {
        use super::Region;
        use std::rc::Rc;
        let counter = Rc::new(());
        let region = Region::new(4096);
        {
            let _scope = region.enter();
            let a = P::new([1u64, 2]);
            let a_addr = a.addr();
            unsafe { free!(a) };
            let b = P::new_recycled([3u64, 4]);
            assert_eq!(b.addr(), a_addr);
            // A value that needs drop is not recycled in a region: its drop entry stays.
            let c = P::new(Rc::clone(&counter));
            let c_addr = c.addr();
            unsafe { free!(c) };
            let d = P::new_recycled(Rc::clone(&counter));
            assert_ne!(d.addr(), c_addr);
            let cp = arena_checkpoint();
            let _spec = P::new(Rc::clone(&counter));
            arena_rewind(cp);
            assert_eq!(Rc::strong_count(&counter), 4);
        }
        drop(region);
        // The rewound value lost its entry (not dropped, like a thread arena); `c` and `d` were dropped.
        assert_eq!(Rc::strong_count(&counter), 2);
    }

    #[test]
    fn scratch_regions_route_escapes_to_the_outer_target() {
        use super::{enter_thread_arena, escape_scratch, scratch_active, Region};
        let outer = Region::new(4096);
        let scratch = Region::new(4096);
        let _o = outer.enter();
        assert!(!scratch_active() && escape_scratch().is_none());
        {
            let _s = scratch.enter_scratch();
            assert!(scratch_active());
            assert!(Region::containing(P::new(1u64).addr()).unwrap().ptr_eq(&scratch));
            {
                let _e = escape_scratch().unwrap();
                // Already escaped: no second scope.
                assert!(escape_scratch().is_none());
                assert!(Region::containing(P::new(2u64).addr()).unwrap().ptr_eq(&outer));
                // Scratch allocations while escaped.
                assert!(Region::containing(P::new_scratch(3u64).addr()).unwrap().ptr_eq(&scratch));
                assert!(Region::containing(crate::alloc_str_scratch("abc").as_ptr() as usize).unwrap().ptr_eq(&scratch));
                // A scope pushed and popped inside the escape restores the escape's target, not the scratch region.
                drop(enter_thread_arena());
                assert!(Region::containing(P::new(4u64).addr()).unwrap().ptr_eq(&outer));
            }
            assert!(Region::containing(P::new(5u64).addr()).unwrap().ptr_eq(&scratch));
        }
        assert!(!scratch_active());
        assert!(Region::containing(P::new(6u64).addr()).unwrap().ptr_eq(&outer));
        // Without a scratch region, `new_scratch` is `new`.
        assert!(Region::containing(P::new_scratch(7u64).addr()).unwrap().ptr_eq(&outer));
    }

    /// A retired region's range is never handed out again (fileregions.rs in tsrs_compiler frees a checked file's
    /// tree while caches keyed by the address of a node in it may keep stale entries): no later chunk, of any
    /// arena or region on any thread, overlaps it.
    #[cfg(compressed_ptrs)]
    #[test]
    fn retired_regions_are_never_reused() {
        use super::Region;
        // More than a quarter of a slab: the region's chunk is a slab of its own, which a plain free would give back
        // to the reservation (or the slab cache) for reuse.
        const SIZE: usize = 1 << 20;
        let retired = Region::new(SIZE);
        let first = {
            let _s = retired.enter();
            P::new([1u64; 8]).addr()
        };
        let range = first..first + retired.allocated_bytes();
        retired.retire_on_free();
        drop(retired);
        let mut later = Vec::new();
        for i in 0..32u64 {
            let r = Region::new(SIZE);
            {
                let _s = r.enter();
                let p = P::new([i; 8]);
                assert!(!range.contains(&p.addr()), "a retired region's range was reused");
            }
            later.push(r);
        }
        let big = crate::alloc_slice(&vec![0u8; 4 * SIZE]);
        assert!(!range.contains(&(big.as_ptr() as usize)) && !range.contains(&(big.as_ptr() as usize + big.len() - 1)));
    }

    #[test]
    fn small_regions_share_slabs_and_trim() {
        use super::Region;
        let a = Region::new(64 << 10);
        let x = {
            let _s = a.enter();
            P::new([7u64; 4])
        };
        a.trim();
        assert!(a.allocated_bytes() < 1024);
        let b = Region::new(64 << 10);
        let y = {
            let _s = b.enter();
            P::new([8u64; 4])
        };
        // Carved right after `a`'s trimmed chunk.
        assert!(y.addr() > x.addr() && y.addr() - x.addr() < 1024);
        assert_eq!((*x, *y), ([7; 4], [8; 4]));
        drop(a);
        assert!(Region::containing(y.addr()).unwrap().ptr_eq(&b));
        assert_eq!(*y, [8; 4]);
    }
}

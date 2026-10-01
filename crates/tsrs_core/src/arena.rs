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

use std::alloc::Layout;
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU8, Ordering};

const FIRST_CHUNK: usize = 1 << 20;
const CHUNK_ALIGN: usize = 16;
const PAGE: usize = 4096;

/// Largest block size kept on a free list; classes are `size / 8`.
pub const MAX_FREE_SIZE: usize = 512;
const CLASSES: usize = MAX_FREE_SIZE / 8 + 1;

/// Fill byte for freed and rewound memory in poison mode (and in debug builds).
pub const POISON: u8 = 0xA5;

pub struct Arena {
    /// Bump finger in the current chunk (allocation moves it down towards `start`).
    ptr: Cell<*mut u8>,
    start: Cell<*mut u8>,
    end: Cell<*mut u8>,
    /// Older chunks: (start, end, finger when the chunk was retired).
    retired: RefCell<Vec<(usize, usize, usize)>>,
    capacity: Cell<usize>,
    /// Bumped by every free and every `pin`; a rewind is skipped when it changed since the checkpoint.
    epoch: Cell<u64>,
    free: [Cell<*mut u8>; CLASSES],
}

/// The bump position of the current thread's arena (`checkpoint`).
#[derive(Clone, Copy)]
pub struct Checkpoint {
    ptr: *mut u8,
    start: *mut u8,
    epoch: u64,
    #[cfg(feature = "alloc-profile")]
    census_blocks: usize,
}

impl Arena {
    pub(crate) fn new() -> Arena {
        let a = Arena {
            ptr: Cell::new(std::ptr::null_mut()),
            start: Cell::new(std::ptr::null_mut()),
            end: Cell::new(std::ptr::null_mut()),
            retired: RefCell::new(Vec::new()),
            capacity: Cell::new(0),
            epoch: Cell::new(0),
            free: [const { Cell::new(std::ptr::null_mut()) }; CLASSES],
        };
        a.new_chunk(FIRST_CHUNK);
        a
    }

    #[inline(always)]
    pub(crate) fn alloc_layout(&self, layout: Layout) -> NonNull<u8> {
        // Profile builds: a zero-sized value (a closure without captures, e.g. a `TypeComparer`) would get the bump
        // position, which is the start of the previous block, and the census would count that word as a reference.
        #[cfg(feature = "alloc-profile")]
        if layout.size() == 0 {
            return NonNull::new(std::ptr::without_provenance_mut(layout.align())).unwrap();
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

    #[cold]
    #[inline(never)]
    fn alloc_layout_slow(&self, layout: Layout) -> NonNull<u8> {
        let prev = self.end.get().addr() - self.start.get().addr();
        let need = layout.size().checked_add(layout.align()).expect("arena allocation size overflow");
        self.new_chunk((prev * 2).max(need));
        let ptr = self.ptr.get();
        let new = (ptr.addr() - layout.size()) & !(layout.align() - 1);
        debug_assert!(new >= self.start.get().addr());
        let p = ptr.with_addr(new);
        self.ptr.set(p);
        // SAFETY: inside the fresh chunk.
        unsafe { NonNull::new_unchecked(p) }
    }

    fn new_chunk(&self, size: usize) {
        let size = size.div_ceil(PAGE) * PAGE;
        let layout = Layout::from_size_align(size, CHUNK_ALIGN).expect("arena chunk layout");
        #[cfg(feature = "alloc-profile")]
        let base = census_chunk(size);
        #[cfg(not(feature = "alloc-profile"))]
        // SAFETY: non-zero size.
        let base = unsafe { std::alloc::alloc(layout) };
        if base.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        // Blocks given back by address (`free_block`) are written through this provenance.
        let _ = base.expose_provenance();
        if !self.start.get().is_null() {
            self.retired.borrow_mut().push((self.start.get().addr(), self.end.get().addr(), self.ptr.get().addr()));
        }
        self.start.set(base);
        // SAFETY: one past the end of the allocation.
        self.end.set(unsafe { base.add(size) });
        self.ptr.set(self.end.get());
        self.capacity.set(self.capacity.get() + size);
    }

    #[inline(always)]
    pub(crate) fn alloc<T>(&self, value: T) -> &mut T {
        let p = self.alloc_layout(Layout::new::<T>()).cast::<T>();
        // SAFETY: fresh, aligned, exclusively owned memory for one `T`.
        unsafe {
            p.as_ptr().write(value);
            &mut *p.as_ptr()
        }
    }

    #[inline]
    pub(crate) fn alloc_slice_copy<T: Copy>(&self, src: &[T]) -> &mut [T] {
        let p = self.alloc_layout(Layout::for_value(src)).cast::<T>();
        // SAFETY: fresh memory for `src.len()` items; `T: Copy`.
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), p.as_ptr(), src.len());
            std::slice::from_raw_parts_mut(p.as_ptr(), src.len())
        }
    }

    #[inline]
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
    pub(crate) fn alloc_str(&self, s: &str) -> &mut str {
        let bytes = self.alloc_slice_copy(s.as_bytes());
        // SAFETY: copied from a `str`.
        unsafe { std::str::from_utf8_unchecked_mut(bytes) }
    }

    /// Total size of the chunks (like bumpalo's `allocated_bytes`).
    pub(crate) fn capacity(&self) -> usize {
        self.capacity.get()
    }

    /// The used part of every chunk, (start, len).
    pub(crate) fn used_ranges(&self) -> Vec<(usize, usize)> {
        let mut out = vec![(self.ptr.get().addr(), self.end.get().addr() - self.ptr.get().addr())];
        out.extend(self.retired.borrow().iter().map(|&(_, end, finger)| (finger, end - finger)));
        out
    }

    #[inline]
    pub(crate) fn pop_free(&self, class: usize) -> Option<NonNull<u8>> {
        let head = self.free[class].get();
        if head.is_null() {
            return None;
        }
        // SAFETY: free blocks hold the next pointer in their first word.
        unsafe {
            self.free[class].set(*(head as *const *mut u8));
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
        std::ptr::write_bytes(p.add(8), POISON, class * 8 - 8);
        *(p as *mut *mut u8) = self.free[class].get();
        self.free[class].set(p);
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
            #[cfg(feature = "alloc-profile")]
            census_blocks: crate::alloc_profile::census_block_count(),
        }
    }

    /// The range allocated since `cp` when it can be discarded: same chunk and no free or pin since.
    #[inline]
    fn rewindable(&self, cp: &Checkpoint) -> Option<(*mut u8, usize)> {
        if cp.start != self.start.get() || cp.epoch != self.epoch.get() {
            return None;
        }
        let now = self.ptr.get();
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

/// Census only: clears `N` bytes of the stack below the caller. The frames that just used the freed or rewound
/// memory are dead but their words stay on the stack, and the padding of the next objects built there copies them
/// into the arena, where the conservative census would count them as references.
#[cfg(feature = "alloc-profile")]
#[inline(never)]
fn scrub_stack<const N: usize>() {
    let mut buf = [0u8; N];
    std::hint::black_box(&mut buf);
}

/// 0 unknown, 1 normal, 2 poison (`TSRS_ARENA_POISON=1`): freed and rewound memory is filled and never reused.
static MODE: AtomicU8 = AtomicU8::new(0);

#[inline]
fn poison_mode() -> bool {
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
#[inline(always)]
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
pub(crate) unsafe fn free_block(arena: &Arena, addr: usize, size: usize, align: usize) {
    let class = free_class(size, align);
    if class == 0 || addr % 8 != 0 {
        return;
    }
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
        std::ptr::write_bytes(p, POISON, size);
        return;
    }
    arena.push_free(class, p);
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
        assert_ne!(s.as_ptr() as usize, addr);
        // SAFETY: `s` is not used again.
        unsafe { free_slice!(s) };
        let t = alloc_slice_recycled(&[7u64, 8]);
        assert_eq!(t.as_ptr() as usize, s.as_ptr() as usize);
        assert_eq!(t, &[7, 8]);
    }

    #[test]
    fn rewind_discards_speculative_allocations_unless_pinned() {
        let before = P::new(1u64);
        let cp = arena_checkpoint();
        let spec = P::new(2u64);
        arena_rewind(cp);
        let after = P::new(3u64);
        assert_eq!(after.addr(), spec.addr());
        assert_eq!((*before, *after), (1, 3));

        let cp = arena_checkpoint();
        let kept = P::new(4u64);
        arena_pin();
        arena_rewind(cp);
        let next = P::new(5u64);
        assert_ne!(next.addr(), kept.addr());
        assert_eq!(*kept, 4);
    }
}

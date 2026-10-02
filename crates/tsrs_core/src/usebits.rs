//! Use census (`TSRS_CENSUS=1 TSRS_USE_CENSUS=1` in the alloc-profile build; compiled to no-ops otherwise): which
//! arena objects are ever read after they are created. notes/mem-use-census.md.
//!
//! A read sets a bit in a bitmap over the profile build's arena address range (chunks are mapped from
//! `arena::CENSUS_ARENA_BASE` up, see `arena::census_chunk`), one bit per 4 bytes, at the address of the field that
//! was read. At the end of the run the census (`alloc_profile::census`) counts a block as read if any bit inside it
//! is set. What counts as a read is decided by the accessors that call `mark`:
//!
//! - `get`-family calls on the cells that hold arena fields (`ucell::Cell::get` / `replace` / `take`,
//!   `ucell::RefCell::borrow` / `try_borrow` / `replace` / `take`, the packed cells of `ptr` and `frozen`), and the
//!   getters of hand-packed words (type symbol word, symbol parent word, value-symbol links), through `mark`;
//! - a slice or string handed out by such a getter: the block that holds the elements (`mark_slice`);
//! - symbol table lookups and iteration (the table header), `TypeMapper::data` (the mapper).
//!
//! Not reads: writes (`set`, `borrow_mut`, the mode checks inside setters, which use `peek`), identity comparison and
//! hashing of `P` (addresses only), object ids (`Type.id`, symbol ids: what cache keys are built from), and the
//! census itself (it reads memory directly, after `freeze`).

#[cfg(feature = "alloc-profile")]
mod imp {
    use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

    /// Start of the profile build's arena mappings (`arena::census_chunk`).
    pub const BASE: usize = 0x7c00_0000_0000;
    /// Address range covered by the bitmap (256 GiB of arena mappings).
    pub const SPAN: usize = 1 << 38;
    const BITMAP_BYTES: usize = SPAN / 32;

    /// The bitmap while reads are recorded; null before `init` and after `freeze`.
    static LIVE: AtomicPtr<AtomicU32> = AtomicPtr::new(std::ptr::null_mut());
    /// The bitmap, kept for the census after `freeze`.
    static FROZEN: AtomicPtr<AtomicU32> = AtomicPtr::new(std::ptr::null_mut());
    /// Reads made inside a builder scope (`builder()`), same layout.
    static BUILDER_MAP: AtomicPtr<AtomicU32> = AtomicPtr::new(std::ptr::null_mut());
    /// Writes (`mark_write`), same layout: link records that were never written could have stayed unallocated.
    static WRITE_MAP: AtomicPtr<AtomicU32> = AtomicPtr::new(std::ptr::null_mut());
    static WRITES_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    thread_local! {
        static BUILDER_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    /// See `super::builder`.
    pub struct Builder(());

    impl Drop for Builder {
        #[inline]
        fn drop(&mut self) {
            let _ = BUILDER_DEPTH.try_with(|d| d.set(d.get() - 1));
        }
    }

    #[inline]
    pub fn builder() -> Builder {
        let _ = BUILDER_DEPTH.try_with(|d| d.set(d.get() + 1));
        Builder(())
    }

    /// Maps the bitmap (lazily committed) and starts recording. Called by the census when `TSRS_USE_CENSUS=1`.
    pub fn init() {
        if !FROZEN.load(Ordering::Relaxed).is_null() {
            return;
        }
        extern "C" {
            fn mmap(addr: *mut std::ffi::c_void, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut std::ffi::c_void;
        }
        const PROT_READ: i32 = 1;
        const PROT_WRITE: i32 = 2;
        const MAP_PRIVATE: i32 = 2;
        const MAP_ANON: i32 = 0x1000;
        // SAFETY: anonymous private mappings, zero-filled on first touch.
        let p = unsafe { mmap(std::ptr::null_mut(), BITMAP_BYTES, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0) };
        let q = unsafe { mmap(std::ptr::null_mut(), BITMAP_BYTES, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0) };
        let r = unsafe { mmap(std::ptr::null_mut(), BITMAP_BYTES, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0) };
        if p.addr() == usize::MAX || q.addr() == usize::MAX || r.addr() == usize::MAX {
            return;
        }
        WRITE_MAP.store(r.cast(), Ordering::Relaxed);
        WRITES_ON.store(true, Ordering::Relaxed);
        BUILDER_MAP.store(q.cast(), Ordering::Relaxed);
        FROZEN.store(p.cast(), Ordering::Relaxed);
        LIVE.store(p.cast(), Ordering::Release);
    }

    /// Stops recording; `is_read` keeps working.
    pub fn freeze() {
        LIVE.store(std::ptr::null_mut(), Ordering::SeqCst);
        WRITES_ON.store(false, Ordering::SeqCst);
    }

    /// Records a write of the 4 bytes at `addr` (ignored outside the arena range).
    #[inline(always)]
    pub fn mark_write(addr: usize) {
        let off = addr.wrapping_sub(BASE);
        if off < SPAN && WRITES_ON.load(Ordering::Relaxed) {
            let bm = WRITE_MAP.load(Ordering::Relaxed);
            let bit = 1u32 << ((off >> 2) & 31);
            // SAFETY: `off < SPAN`, so the word lies inside the mapping.
            let w = unsafe { &*bm.add(off >> 7) };
            if w.load(Ordering::Relaxed) & bit == 0 {
                w.fetch_or(bit, Ordering::Relaxed);
            }
        }
    }

    /// Whether any 4-byte granule of `[addr, addr + size)` was written.
    pub fn is_written(addr: usize, size: usize) -> bool {
        any_bit(WRITE_MAP.load(Ordering::Relaxed), addr, size)
    }

    pub fn recording() -> bool {
        !LIVE.load(Ordering::Relaxed).is_null()
    }

    /// Whether the use census was started (`init`), recording or frozen.
    pub fn is_active() -> bool {
        !FROZEN.load(Ordering::Relaxed).is_null()
    }

    /// Records a read of the 4 bytes at `addr` (ignored outside the arena range).
    #[inline(always)]
    pub fn mark(addr: usize) {
        let off = addr.wrapping_sub(BASE);
        if off < SPAN {
            mark_slow(off);
        }
    }

    #[inline(never)]
    fn mark_slow(off: usize) {
        let mut bm = LIVE.load(Ordering::Relaxed);
        if bm.is_null() {
            return;
        }
        if BUILDER_DEPTH.try_with(|d| d.get()).unwrap_or(0) != 0 {
            bm = BUILDER_MAP.load(Ordering::Relaxed);
            let bit = 1u32 << ((off >> 2) & 31);
            // SAFETY: `off < SPAN`, so the word lies inside the mapping.
            let w = unsafe { &*bm.add(off >> 7) };
            if w.load(Ordering::Relaxed) & bit == 0 {
                w.fetch_or(bit, Ordering::Relaxed);
            }
            return;
        }
        let bit = 1u32 << ((off >> 2) & 31);
        // SAFETY: `off < SPAN`, so the word lies inside the mapping.
        let w = unsafe { &*bm.add(off >> 7) };
        if w.load(Ordering::Relaxed) & bit == 0 && w.fetch_or(bit, Ordering::Relaxed) & bit == 0 && FIRST_READS.load(Ordering::Relaxed) {
            crate::alloc_profile::census::sample_first_read(BASE + off);
        }
    }

    static FIRST_READS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// `TSRS_USE_CENSUS_FIRST_READS=1`: sample the stacks of first reads (see `census::sample_first_read`).
    pub fn set_first_reads(on: bool) {
        FIRST_READS.store(on, Ordering::Relaxed);
    }

    /// Whether any 4-byte granule of `[addr, addr + size)` was read (after `freeze`; false outside the range).
    pub fn is_read(addr: usize, size: usize) -> bool {
        any_bit(FROZEN.load(Ordering::Relaxed), addr, size)
    }

    /// Whether any 4-byte granule of `[addr, addr + size)` was read inside a builder scope.
    pub fn is_builder_read(addr: usize, size: usize) -> bool {
        any_bit(BUILDER_MAP.load(Ordering::Relaxed), addr, size)
    }

    fn any_bit(bm: *mut AtomicU32, addr: usize, size: usize) -> bool {
        let off = addr.wrapping_sub(BASE);
        if bm.is_null() || off >= SPAN || size == 0 {
            return false;
        }
        let (first, last) = (off >> 2, (off + size - 1).min(SPAN - 1) >> 2);
        let mut g = first;
        while g <= last {
            // SAFETY: inside the mapping (`g < SPAN / 4`).
            let w = unsafe { (*bm.add(g >> 5)).load(Ordering::Relaxed) };
            if g & 31 == 0 && last - g >= 31 {
                if w != 0 {
                    return true;
                }
                g += 32;
                continue;
            }
            if w & (1 << (g & 31)) != 0 {
                return true;
            }
            g += 1;
        }
        false
    }
}

#[cfg(feature = "alloc-profile")]
pub use imp::{builder, freeze, init, is_active, is_builder_read, is_read, is_written, mark, mark_write, recording, set_first_reads, Builder};

#[cfg(not(feature = "alloc-profile"))]
#[inline(always)]
pub fn mark(_addr: usize) {}

#[cfg(not(feature = "alloc-profile"))]
#[inline(always)]
pub fn mark_write(_addr: usize) {}

/// A builder scope: until the guard is dropped, reads on this thread are recorded as builder reads, not uses. For
/// code that only assembles a structure from objects (sorting and filtering a member list, copying inherited
/// members into a table, instantiating a symbol from its declaration): an object that is only ever touched by
/// builders is in a structure but never consulted.
#[cfg(not(feature = "alloc-profile"))]
pub struct Builder;

#[cfg(not(feature = "alloc-profile"))]
#[inline(always)]
pub fn builder() -> Builder {
    Builder
}

/// A link record handed out for the first time (`addr`, `size` bytes, value type `ty`, owner object `key`, 0 if
/// unknown): the use census reports link records per record, not per chunk.
#[inline]
pub fn note_slot(_addr: usize, _size: usize, _ty: &'static str, _key: usize) {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::note_slot(_addr, _size, _ty, _key);
}

/// The checker phase starts (first `new_checker`): the use census reports objects allocated from here on
/// separately from the front end's (parse, bind).
#[inline]
pub fn note_check_start() {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::note_check_start();
}

/// Records a read of the value at `p` (see the module docs).
#[inline(always)]
pub fn mark_ptr<T: ?Sized>(p: *const T) {
    mark(p as *const () as usize)
}

/// A value of 16 bytes whose first word points into the arena and whose second is a small length is taken to be a
/// slice or string: handing it out reads the block that holds the elements. Other values: nothing.
#[inline(always)]
pub fn mark_value<T>(v: &T) {
    if std::mem::size_of::<T>() == 16 && std::mem::align_of::<T>() == 8 {
        // SAFETY: a 16-byte, 8-aligned value; only its two words are inspected.
        let w = unsafe { *(v as *const T as *const [usize; 2]) };
        if w[1] < 1 << 32 {
            mark(w[0]);
        }
    } else if std::mem::size_of::<T>() == 24 && std::mem::align_of::<T>() == 8 {
        // An enum with a slice or string payload after its tag (`LiteralValue::String`, `PseudoBigInt`).
        // SAFETY: a 24-byte, 8-aligned value; only its three words are inspected.
        let w = unsafe { *(v as *const T as *const [usize; 3]) };
        if w[2] < 1 << 32 {
            mark(w[1]);
        }
    }
}

/// Records a use of the elements of `s` and returns it (for slices held outside cells, such as lazy member tables).
#[inline(always)]
pub fn used<T>(s: &'static [T]) -> &'static [T] {
    mark_slice(s);
    s
}

/// Records that the elements of `s` were handed out (the block holding them is read).
#[inline(always)]
pub fn mark_slice<T>(s: &[T]) {
    mark(s.as_ptr() as usize)
}

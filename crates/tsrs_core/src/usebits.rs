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
        unsafe extern "C" {
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
        if EPOCH_ON.load(Ordering::Relaxed) {
            epoch_note_read(off);
        }
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

    // ---- Epoch census (experiment, notes/mem-recycle-checkers.md) -----------------------------------------------
    // Per 16-byte granule of the first 64 GiB of arena mappings: the allocating checker thread and its epoch (files
    // that thread had finished; 0 = not a checker allocation), and the latest epoch at which the granule was read.
    const E_SPAN: usize = 1 << 36;
    const E_GRAN: usize = 16;
    static EPOCH_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static ALLOC_MAP: AtomicPtr<AtomicU32> = AtomicPtr::new(std::ptr::null_mut());
    static READ_MAP: AtomicPtr<AtomicU32> = AtomicPtr::new(std::ptr::null_mut());
    static SITE_MAP: AtomicPtr<AtomicU32> = AtomicPtr::new(std::ptr::null_mut());
    thread_local! {
        static EPOCH: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((0, 0)) };
    }
    const START: u32 = 1 << 31;
    const EPOCH_BITS: u32 = 27;

    pub fn epoch_init() {
        unsafe extern "C" {
            fn mmap(addr: *mut std::ffi::c_void, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut std::ffi::c_void;
        }
        #[cfg(target_os = "linux")]
        const MAP_ANON: i32 = 0x20;
        #[cfg(not(target_os = "linux"))]
        const MAP_ANON: i32 = 0x1000;
        let bytes = E_SPAN / E_GRAN * 4;
        // SAFETY: anonymous private mappings, zero-filled on first touch.
        let a = unsafe { mmap(std::ptr::null_mut(), bytes, 3, 2 | MAP_ANON, -1, 0) };
        let r = unsafe { mmap(std::ptr::null_mut(), bytes, 3, 2 | MAP_ANON, -1, 0) };
        assert!(a.addr() != usize::MAX && r.addr() != usize::MAX, "epoch census maps");
        let st = unsafe { mmap(std::ptr::null_mut(), bytes, 3, 2 | MAP_ANON, -1, 0) };
        assert!(st.addr() != usize::MAX);
        SITE_MAP.store(st.cast(), Ordering::Relaxed);
        ALLOC_MAP.store(a.cast(), Ordering::Relaxed);
        READ_MAP.store(r.cast(), Ordering::Relaxed);
        EPOCH_ON.store(true, Ordering::SeqCst);
    }

    /// The calling checker thread `thread` (1-based) has finished `epoch - 1` files.
    pub fn epoch_set(thread: u32, epoch: u32) {
        EPOCH.with(|e| e.set((thread, epoch)));
    }

    #[inline]
    pub fn epoch_note_alloc(addr: usize, size: usize) {
        if !EPOCH_ON.load(Ordering::Relaxed) {
            return;
        }
        let (thread, epoch) = EPOCH.with(|e| e.get());
        if epoch == 0 || size == 0 {
            return;
        }
        let off = addr.wrapping_sub(BASE);
        if off >= E_SPAN {
            return;
        }
        let m = ALLOC_MAP.load(Ordering::Relaxed);
        let first = off / E_GRAN;
        let last = (off + size - 1).min(E_SPAN - 1) / E_GRAN;
        let v = thread << EPOCH_BITS | epoch;
        for g in first..=last {
            // SAFETY: inside the mapping.
            unsafe { (*m.add(g)).store(if g == first { v | START } else { v }, Ordering::Relaxed) };
        }
    }

    #[inline]
    pub fn epoch_note_site(addr: usize, site: u32) {
        if !EPOCH_ON.load(Ordering::Relaxed) {
            return;
        }
        let off = addr.wrapping_sub(BASE);
        if off < E_SPAN {
            // SAFETY: inside the mapping.
            unsafe { (*SITE_MAP.load(Ordering::Relaxed).add(off / E_GRAN)).store(site, Ordering::Relaxed) };
        }
    }

    #[inline]
    fn epoch_note_read(off: usize) {
        if off >= E_SPAN {
            return;
        }
        let epoch = EPOCH.with(|e| e.get().1);
        if epoch == 0 {
            return;
        }
        // SAFETY: inside the mapping.
        let w = unsafe { &*READ_MAP.load(Ordering::Relaxed).add(off / E_GRAN) };
        if w.load(Ordering::Relaxed) < epoch {
            w.fetch_max(epoch, Ordering::Relaxed);
        }
    }

    /// Prints, per checker thread and for points t through its life (fractions of its final epoch), the bytes it had
    /// allocated by t, and of those the bytes never read after t (or never read at all).
    pub fn epoch_report(final_epochs: &[u32]) {
        if !EPOCH_ON.load(Ordering::Relaxed) {
            return;
        }
        let a = ALLOC_MAP.load(Ordering::Relaxed);
        let r = READ_MAP.load(Ordering::Relaxed);
        let used = crate::arena::census_mapped_end().saturating_sub(BASE).min(E_SPAN);
        const POINTS: [f64; 9] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
        let n = final_epochs.len();
        // [thread][point]: (allocated by t, never read after t)
        let mut acc = vec![[(0u64, 0u64); 9]; n + 1];
        let mut never = vec![0u64; n + 1];
        let sm = SITE_MAP.load(Ordering::Relaxed);
        let mut by_site: rustc_hash::FxHashMap<u32, (u64, u64)> = Default::default();
        let mut total = vec![0u64; n + 1];
        let granules = used / E_GRAN;
        let mut g = 0;
        while g < granules {
            // SAFETY: inside the mapping.
            let v = unsafe { (*a.add(g)).load(Ordering::Relaxed) };
            if v & START == 0 {
                g += 1;
                continue;
            }
            let thread = ((v & !START) >> EPOCH_BITS) as usize;
            let epoch = v & ((1 << EPOCH_BITS) - 1);
            let mut last_read = unsafe { (*r.add(g)).load(Ordering::Relaxed) };
            let mut h = g + 1;
            while h < granules {
                let w = unsafe { (*a.add(h)).load(Ordering::Relaxed) };
                if w == 0 || w & START != 0 || w & !START != v & !START {
                    break;
                }
                last_read = last_read.max(unsafe { (*r.add(h)).load(Ordering::Relaxed) });
                h += 1;
            }
            let bytes = ((h - g) * E_GRAN) as u64;
            if thread == 0 || thread > n {
                g = h;
                continue;
            }
            total[thread] += bytes;
            if last_read == 0 {
                never[thread] += bytes;
            }
            let fin = final_epochs[thread - 1].max(1);
            for (k, f) in POINTS.iter().enumerate() {
                let t = (f * fin as f64) as u32;
                if epoch <= t {
                    acc[thread][k].0 += bytes;
                    if last_read <= t {
                        acc[thread][k].1 += bytes;
                    }
                    if k == 4 {
                        let site = unsafe { (*sm.add(g)).load(Ordering::Relaxed) };
                        let e = by_site.entry(site).or_default();
                        e.0 += bytes;
                        if last_read <= t {
                            e.1 += bytes;
                        }
                    }
                }
            }
            g = h;
        }
        let mb = |b: u64| b as f64 / 1048576.0;
        eprintln!("epoch census: per checker thread, at t = fraction of its files: MB allocated by t / of which never read after t (%)");
        let mut sum = [(0u64, 0u64); 9];
        for t in 1..=n {
            let mut line = format!("  checker {} ({} files, {:.0} MB, never read {:.0}%):", t - 1, final_epochs[t - 1], mb(total[t]), 100.0 * never[t] as f64 / total[t].max(1) as f64);
            for k in 0..9 {
                let (al, dead) = acc[t][k];
                sum[k].0 += al;
                sum[k].1 += dead;
                line += &format!(" {:.1}:{:.0}/{:.0}%", POINTS[k], mb(al), 100.0 * dead as f64 / al.max(1) as f64);
            }
            eprintln!("{line}");
        }
        let mut line = String::from("  all:");
        for k in 0..9 {
            line += &format!(" {:.1}:{:.0}/{:.0}({:.0}%)", POINTS[k], mb(sum[k].0), mb(sum[k].1), 100.0 * sum[k].1 as f64 / sum[k].0.max(1) as f64);
        }
        eprintln!("{line}");
        let names = crate::alloc_profile::GLOBAL_SITES.lock().unwrap();
        let mut agg: rustc_hash::FxHashMap<&str, (u64, u64)> = Default::default();
        for (site, (al, dead)) in by_site {
            let name = if site == 0 { "?" } else { names.get(site as usize - 1).map_or("?", |s| s.as_str()) };
            let e = agg.entry(name).or_default();
            e.0 += al;
            e.1 += dead;
        }
        let mut v: Vec<_> = agg.into_iter().collect();
        v.sort_by_key(|&(_, (_, dead))| std::cmp::Reverse(dead));
        eprintln!("epoch census at t = 0.5, by allocation site: MB allocated by t, MB never read after t, share");
        for (name, (al, dead)) in v.into_iter().take(40) {
            eprintln!("  {:9.1} {:9.1} {:5.1}%  {}", mb(al), mb(dead), 100.0 * dead as f64 / al.max(1) as f64, name);
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
pub use imp::{epoch_init, epoch_note_alloc, epoch_note_site, epoch_report, epoch_set};
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
        // An enum or struct with a slice or string payload (`LiteralValue::String`, `PseudoBigInt`), at either
        // word: a word followed by a small length is marked (`mark` ignores what is not an arena address).
        // SAFETY: a 24-byte, 8-aligned value; only its three words are inspected.
        let w = unsafe { *(v as *const T as *const [usize; 3]) };
        if w[1] < 1 << 32 {
            mark(w[0]);
        }
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

// ---- GC simulation (experiment, notes/mem-recycle-checkers.md) ------------------------------------------------
// Heap blocks allocated inside a `weak_scope` (the storage of the checker's identity caches) are weak for the census:
// marked if reached, but their words are not followed.
#[cfg(feature = "alloc-profile")]
thread_local! {
    pub(crate) static WEAK_ALLOC: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub struct WeakScope(());

impl Drop for WeakScope {
    #[inline]
    fn drop(&mut self) {
        #[cfg(feature = "alloc-profile")]
        let _ = WEAK_ALLOC.try_with(|c| c.set(c.get() - 1));
    }
}

#[inline]
pub fn weak_scope() -> WeakScope {
    #[cfg(feature = "alloc-profile")]
    let _ = WEAK_ALLOC.try_with(|c| c.set(c.get() + 1));
    WeakScope(())
}

/// Runs the reachability census now (alloc-profile build with `TSRS_CENSUS=1`) from `roots`.
pub fn gc_sim_census(roots: &[usize]) {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::run(roots);
    #[cfg(not(feature = "alloc-profile"))]
    let _ = roots;
}

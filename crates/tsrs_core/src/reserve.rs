//! The process-wide address range behind compressed `P<T>` handles (`cfg(compressed_ptrs)`: the default feature
//! `compressed-ptrs` on unix, see build.rs; notes/mem-pointer-compression.md).
//!
//! The first chunk request reserves the range `BASE_ADDR + FIRST .. BASE_ADDR + RESERVE` with no access, and every
//! arena chunk of every thread arena and region is carved from it and committed on hand-out, so a handle turns into
//! an address with no lookup: `at(handle << UNIT_SHIFT)`. Offsets below `FIRST` are never handed out, so handle 0 is
//! never an object (the `Option<P<T>>` niche). Released chunks (region slabs) are decommitted and their ranges reused.
//!
//! Linux: thread-arena chunks from 2 MiB up (`alloc_chunk` with `huge`) are whole, aligned 2 MiB blocks advised
//! with `MADV_HUGEPAGE`, so the kernel backs them with transparent huge pages where THP is `madvise` or `always`
//! (notes/linux-x86-round.md). Region slabs are released piecemeal and keep 4 KiB pages.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Handles count units of `1 << UNIT_SHIFT` bytes, so every `P` target is 8-aligned.
pub const UNIT_SHIFT: u32 = 3;
/// 2^32 units: 32 GiB.
pub const RESERVE: usize = 1 << (32 + UNIT_SHIFT);
/// Chunk sizes and offsets are multiples of this (a multiple of every supported page size).
const GRANULE: usize = 64 << 10;
/// Size and alignment of `huge` chunks: on Linux the transparent huge page of a 4 KiB-page kernel (x86-64, arm64). A
/// fault gets a huge page only if the aligned 2 MiB around it lies inside one advised read-write mapping, so a chunk
/// must not share a 2 MiB block with a neighbour. Elsewhere `GRANULE` (macOS has no huge pages for anonymous memory).
#[cfg(target_os = "linux")]
pub const HUGE_CHUNK: usize = 2 << 20;
#[cfg(not(target_os = "linux"))]
pub const HUGE_CHUNK: usize = GRANULE;

/// Where the reservation lives: a fixed address per target, so turning a handle into an address costs no load (a
/// base loaded from a global cost ~25% more instructions; notes/mem-pointer-compression.md sections 4-6).
///
/// Linux: 0, "zero-based" handles (the JVM's zero-based compressed oops): the address is `handle << 3`, which x86-64
/// folds into the addressing mode (`[idx*8 + disp]`) and arm64 into a shifted-register load or one `ubfiz`. A base
/// that does not fit a 32-bit displacement costs ~7% instructions and wall time on x86-64 (section 6). The range
/// `FIRST .. RESERVE` (4-32 GiB) is free in every Linux process seen: PIE executables, their brk heap, shared
/// libraries and default mmap placements are all above 2^40, and non-PIE executables load at 4 MiB.
///
/// Elsewhere (macOS): 64 TiB + 4 GiB. macOS keeps 4-448 GiB for the binary, the shared cache and its malloc zones
/// (the low 4 GiB is `__PAGEZERO`), so no low 32 GiB range exists there; on arm64 the base costs about one `add` per
/// pointer chase and no wall time (section 5). The value is chosen for arm64 codegen: one `movz` materializes it, and
/// bit 32 overlaps the offset range (`handle << 3` < 2^35), so LLVM cannot turn the add into an `orr`; `add x, base,
/// w, uxtw #3` then folds the zero-extension and the shift of a 32-bit handle into the one instruction. It needs a
/// 47-bit address space (arm64 with 48-bit VA).
#[cfg(target_os = "linux")]
pub const BASE_ADDR: usize = 0;
#[cfg(not(target_os = "linux"))]
pub const BASE_ADDR: usize = 0x4001_0000_0000;

/// Offset of the first byte ever handed out; handles below `FIRST >> UNIT_SHIFT` never name an object. Zero-based,
/// 4 GiB: clear of non-PIE executables and their brk heap (from 4 MiB), of `MAP_32BIT` and other low-2-GiB users, and
/// every arena address stays above `u32::MAX`, as with a high base. That leaves 28 GiB (the worst measured run uses
/// 6 GB). With a high base only the first granule is skipped.
#[cfg(target_os = "linux")]
pub const FIRST: usize = 4 << 30;
#[cfg(not(target_os = "linux"))]
pub const FIRST: usize = GRANULE;

static RESERVED: AtomicBool = AtomicBool::new(false);

/// The address of offset 0 of the reservation (not itself mapped: offsets below `FIRST` are never handed out).
#[inline]
pub fn base() -> *mut u8 {
    std::ptr::with_exposed_provenance_mut(BASE_ADDR)
}

/// The address at byte offset `off` of the reservation.
///
/// # Safety
/// `off` must lie inside a chunk handed out by `alloc_chunk` (or be its end).
#[expect(clippy::inline_always, reason = "every `P` dereference goes through it; it must fold into the load")]
#[inline(always)]
pub unsafe fn at(off: usize) -> *mut u8 {
    if BASE_ADDR == 0 {
        // Zero-based: the offset is the address. `base().add(off)` would offset a null pointer, which is UB.
        std::ptr::with_exposed_provenance_mut(off)
    } else {
        // SAFETY: `base() + off` lies inside the reservation, whose provenance `reserve` exposed.
        unsafe { base().add(off) }
    }
}

struct Chunks {
    /// Offset of the first never-used byte.
    next: usize,
    /// Released ranges, offset -> length (coalesced).
    free: BTreeMap<usize, usize>,
    /// Bytes handed out and not released.
    live: usize,
    /// Highest `next` so far.
    peak: usize,
}

static CHUNKS: Mutex<Chunks> = Mutex::new(Chunks { next: FIRST, free: BTreeMap::new(), live: 0, peak: FIRST });

#[cold]
fn fail(what: &str) -> ! {
    let live = CHUNKS.try_lock().map(|c| c.live).unwrap_or(0);
    eprintln!(
        "tsrs: {what} (compressed pointers: one {} GiB address range at {:#x}..{:#x} for every arena; {} MiB of it in \
         use). Build with the tsrs_core/plain-ptrs feature to lift the limit.",
        (RESERVE - FIRST) >> 30,
        BASE_ADDR + FIRST,
        BASE_ADDR + RESERVE,
        live >> 20
    );
    std::process::abort()
}

/// Linux refuses an occupied fixed range (`EEXIST`) instead of placing it elsewhere; kernels before 4.17 treat the
/// flag as a hint, which the address check in `reserve` covers.
#[cfg(target_os = "linux")]
const RESERVE_FLAGS: libc::c_int = libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE | libc::MAP_FIXED_NOREPLACE;
#[cfg(all(unix, not(target_os = "linux")))]
const RESERVE_FLAGS: libc::c_int = libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE;

#[cfg(unix)]
fn reserve() {
    let start = BASE_ADDR + FIRST;
    // SAFETY: a fresh anonymous mapping with no access at a hint (Linux: at exactly that range if it is free);
    // nothing else refers to it.
    let p = unsafe { libc::mmap(std::ptr::without_provenance_mut(start), RESERVE - FIRST, libc::PROT_NONE, RESERVE_FLAGS, -1, 0) };
    if p == libc::MAP_FAILED {
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) {
            fail("the arena address range is taken by another mapping");
        }
        fail("could not reserve the arena address range");
    }
    if p.addr() != start {
        // SAFETY: the mapping just made, unused.
        unsafe { libc::munmap(p, RESERVE - FIRST) };
        fail(&format!("the arena address range is taken (the system placed it at {:#x})", p.addr()));
    }
    // `at` derives every arena pointer from the address.
    let _ = p.expose_provenance();
}

#[cfg(unix)]
fn commit(p: *mut u8, len: usize, huge: bool) {
    // Before `mprotect`, so the range joins an advised read-write neighbour instead of splitting it again. A release
    // (`decommit`) replaces the mapping and drops the advice; the next commit of the range gives it again.
    #[cfg(target_os = "linux")]
    if huge {
        // SAFETY: `p .. p + len` lies inside the reservation. Advice only: it fails on kernels without THP and does
        // nothing where THP is `never`, and either way the chunk works with 4 KiB pages.
        unsafe { libc::madvise(p.cast(), len, libc::MADV_HUGEPAGE) };
    }
    #[cfg(not(target_os = "linux"))]
    let _ = huge;
    // SAFETY: `p .. p + len` lies inside the reservation and is not in use.
    if unsafe { libc::mprotect(p.cast(), len, libc::PROT_READ | libc::PROT_WRITE) } != 0 {
        fail("could not commit arena memory");
    }
}

#[cfg(unix)]
fn decommit(p: *mut u8, len: usize) {
    // A fixed no-access mapping over the range drops its pages (macOS and Linux) and keeps the range reserved.
    // SAFETY: `p .. p + len` lies inside the reservation and nothing uses it any more.
    let q = unsafe {
        libc::mmap(p.cast(), len, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE | libc::MAP_FIXED, -1, 0)
    };
    if q == libc::MAP_FAILED {
        fail("could not release arena memory");
    }
}

impl Chunks {
    /// Offset (a multiple of `align`) of a free range of `size` bytes (both multiples of `GRANULE`): first fit among
    /// the released ranges, else the end. What alignment skips stays free. `None` when the reservation is exhausted.
    fn take(&mut self, size: usize, align: usize) -> Option<usize> {
        let found = self.free.iter().find_map(|(&off, &len)| {
            let start = off.next_multiple_of(align);
            (start + size <= off + len).then_some((off, len, start))
        });
        let start = match found {
            Some((off, len, start)) => {
                self.free.remove(&off);
                if start > off {
                    self.free.insert(off, start - off);
                }
                if off + len > start + size {
                    self.free.insert(start + size, off + len - (start + size));
                }
                start
            }
            None => {
                let start = self.next.next_multiple_of(align);
                if start + size > RESERVE {
                    return None;
                }
                // No free range ends at `next` (`put` lowers `next` instead), so the gap is a range of its own.
                if start > self.next {
                    self.free.insert(self.next, start - self.next);
                }
                self.next = start + size;
                self.peak = self.peak.max(self.next);
                start
            }
        };
        self.live += size;
        Some(start)
    }

    /// Returns the range `off .. off + size` (from `take`), merged with its free neighbours.
    fn put(&mut self, mut off: usize, size: usize) {
        self.live -= size;
        let mut len = size;
        if let Some((&prev, &prev_len)) = self.free.range(..off).next_back() {
            if prev + prev_len == off {
                self.free.remove(&prev);
                off = prev;
                len += prev_len;
            }
        }
        if let Some(next_len) = self.free.remove(&(off + len)) {
            len += next_len;
        }
        if off + len == self.next {
            self.next = off;
        } else {
            self.free.insert(off, len);
        }
    }
}

/// A committed, zero-filled chunk of at least `size` bytes, `GRANULE`-aligned. With `huge` (large thread-arena chunks,
/// which are never released): `HUGE_CHUNK`-aligned, a multiple of it, and on Linux advised for transparent huge pages;
/// callers round `size` to `HUGE_CHUNK` themselves so that they use the whole chunk.
pub(crate) fn alloc_chunk(size: usize, huge: bool) -> *mut u8 {
    let align = if huge { HUGE_CHUNK } else { GRANULE };
    let size = size.next_multiple_of(align);
    let mut c = CHUNKS.lock().unwrap_or_else(|e| e.into_inner());
    if !RESERVED.load(Ordering::Relaxed) {
        reserve();
        RESERVED.store(true, Ordering::Relaxed);
    }
    let Some(off) = c.take(size, align) else {
        drop(c);
        fail("the arena address range is exhausted");
    };
    drop(c);
    // SAFETY: the chunk just taken.
    let p = unsafe { at(off) };
    commit(p, size, huge);
    p
}

/// Gives back a chunk from `alloc_chunk` (same `size`); its memory is released and its range reused.
///
/// # Safety
/// Nothing may use the chunk afterwards.
pub(crate) unsafe fn release_chunk(p: *mut u8, size: usize) {
    let size = size.next_multiple_of(GRANULE);
    decommit(p, size);
    let off = p.addr() - base().addr();
    CHUNKS.lock().unwrap_or_else(|e| e.into_inner()).put(off, size);
}

/// Bytes of the reservation handed out as chunks and not released.
pub fn reserved_in_use() -> usize {
    CHUNKS.lock().unwrap_or_else(|e| e.into_inner()).live
}

/// How far into the reservation chunks have ever been carved, from `FIRST` (released ranges are reused before it
/// grows).
pub fn reserved_high_water() -> usize {
    CHUNKS.lock().unwrap_or_else(|e| e.into_inner()).peak - FIRST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_ranges_are_reused_and_coalesced() {
        let mut c = Chunks { next: FIRST, free: BTreeMap::new(), live: 0, peak: FIRST };
        let a = c.take(GRANULE, GRANULE).unwrap();
        let b = c.take(3 * GRANULE, GRANULE).unwrap();
        let d = c.take(GRANULE, GRANULE).unwrap();
        assert_eq!((a, b, d), (FIRST, FIRST + GRANULE, FIRST + 4 * GRANULE));
        c.put(a, GRANULE);
        c.put(b, 3 * GRANULE);
        assert_eq!(c.free.len(), 1);
        assert_eq!(c.take(4 * GRANULE, GRANULE), Some(a));
        c.put(d, GRANULE);
        assert_eq!(c.next, d);
        assert_eq!(c.live, 4 * GRANULE);
        assert_eq!(c.take(RESERVE, GRANULE), None);
    }

    #[test]
    fn aligned_takes_leave_the_skipped_range_free() {
        const A: usize = 32 * GRANULE;
        let mut c = Chunks { next: GRANULE, free: BTreeMap::new(), live: 0, peak: GRANULE };
        let small = c.take(GRANULE, GRANULE).unwrap();
        let big = c.take(A, A).unwrap();
        assert_eq!((small, big), (GRANULE, A));
        // The gap between them serves unaligned takes first, then aligned ones fit only past `big`.
        assert_eq!(c.free.iter().map(|(&o, &l)| (o, l)).collect::<Vec<_>>(), [(2 * GRANULE, A - 2 * GRANULE)]);
        assert_eq!(c.take(3 * GRANULE, GRANULE), Some(2 * GRANULE));
        assert_eq!(c.take(GRANULE, A), Some(2 * A));
        assert_eq!(c.live, 5 * GRANULE + A);
        // Everything back: the free ranges coalesce and `next` returns to the start.
        c.put(big, A);
        c.put(2 * A, GRANULE);
        c.put(2 * GRANULE, 3 * GRANULE);
        c.put(small, GRANULE);
        assert_eq!((c.next, c.live, c.free.len()), (GRANULE, 0, 0));
        // A released range is reused at its first aligned offset, its unaligned head stays free.
        let x = c.take(A, GRANULE).unwrap();
        let y = c.take(GRANULE, GRANULE).unwrap();
        c.put(x, A);
        assert_eq!(c.take(A / 2, A / 2), Some(A / 2));
        assert_eq!(c.free.iter().map(|(&o, &l)| (o, l)).collect::<Vec<_>>(), [(GRANULE, A / 2 - GRANULE), (A, GRANULE)]);
        assert!(y > x);
    }

    #[test]
    fn chunks_are_committed_and_zeroed() {
        let p = alloc_chunk(1, false);
        // SAFETY: fresh, committed.
        unsafe {
            assert_eq!(*p.add(GRANULE - 1), 0);
            p.write(7);
            release_chunk(p, 1);
        }
        assert!(p.addr() - base().addr() >= FIRST);
        let h = alloc_chunk(HUGE_CHUNK, true);
        assert_eq!((h.addr() - base().addr()) % HUGE_CHUNK, 0);
        // SAFETY: fresh, committed.
        unsafe {
            assert_eq!(*h.add(HUGE_CHUNK - 1), 0);
            h.write(7);
        }
    }
}

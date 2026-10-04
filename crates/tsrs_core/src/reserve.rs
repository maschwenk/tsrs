//! The process-wide address range behind compressed `P<T>` handles (`cfg(compressed_ptrs)`: the default feature
//! `compressed-ptrs` on unix, see build.rs; notes/mem-pointer-compression.md).
//!
//! The first chunk request reserves the range `BASE_ADDR + FIRST .. BASE_ADDR + RESERVE` with no access, and every
//! arena chunk of every thread arena and region is carved from it and committed on hand-out, so a handle turns into
//! an address with no lookup: `at(handle << UNIT_SHIFT)`. Offsets below `FIRST` are never handed out, so handle 0 is
//! never an object (the `Option<P<T>>` niche). Released chunks (region slabs) are decommitted and their ranges reused.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Handles count units of `1 << UNIT_SHIFT` bytes, so every `P` target is 8-aligned.
pub const UNIT_SHIFT: u32 = 3;
/// 2^32 units: 32 GiB.
pub const RESERVE: usize = 1 << (32 + UNIT_SHIFT);
/// Chunk sizes and offsets are multiples of this (a multiple of every supported page size).
const GRANULE: usize = 64 << 10;

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
#[inline(always)]
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
fn commit(p: *mut u8, len: usize) {
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
    /// Offset of a free range of `size` bytes (a multiple of `GRANULE`): first fit among the released ranges, else
    /// the end. `None` when the reservation is exhausted.
    fn take(&mut self, size: usize) -> Option<usize> {
        let found = self.free.iter().find(|&(_, &len)| len >= size).map(|(&off, &len)| (off, len));
        let off = match found {
            Some((off, len)) => {
                self.free.remove(&off);
                if len > size {
                    self.free.insert(off + size, len - size);
                }
                off
            }
            None => {
                let off = self.next;
                if off + size > RESERVE {
                    return None;
                }
                self.next = off + size;
                self.peak = self.peak.max(self.next);
                off
            }
        };
        self.live += size;
        Some(off)
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

/// A committed, zero-filled chunk of at least `size` bytes, `GRANULE`-aligned.
pub(crate) fn alloc_chunk(size: usize) -> *mut u8 {
    let size = size.next_multiple_of(GRANULE);
    let mut c = CHUNKS.lock().unwrap_or_else(|e| e.into_inner());
    if !RESERVED.load(Ordering::Relaxed) {
        reserve();
        RESERVED.store(true, Ordering::Relaxed);
    }
    let Some(off) = c.take(size) else {
        drop(c);
        fail("the arena address range is exhausted");
    };
    drop(c);
    // SAFETY: the chunk just taken.
    let p = unsafe { at(off) };
    commit(p, size);
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
        let a = c.take(GRANULE).unwrap();
        let b = c.take(3 * GRANULE).unwrap();
        let d = c.take(GRANULE).unwrap();
        assert_eq!((a, b, d), (FIRST, FIRST + GRANULE, FIRST + 4 * GRANULE));
        c.put(a, GRANULE);
        c.put(b, 3 * GRANULE);
        assert_eq!(c.free.len(), 1);
        assert_eq!(c.take(4 * GRANULE), Some(a));
        c.put(d, GRANULE);
        assert_eq!(c.next, d);
        assert_eq!(c.live, 4 * GRANULE);
        assert_eq!(c.take(RESERVE), None);
    }

    #[test]
    fn chunks_are_committed_and_zeroed() {
        let p = alloc_chunk(1);
        // SAFETY: fresh, committed.
        unsafe {
            assert_eq!(*p.add(GRANULE - 1), 0);
            p.write(7);
            release_chunk(p, 1);
        }
        assert!(p.addr() - base().addr() >= FIRST);
    }
}

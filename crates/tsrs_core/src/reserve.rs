//! The process-wide address range behind compressed `P<T>` handles (`cfg(compressed_ptrs)`: the default feature
//! `compressed-ptrs` on unix, see build.rs; notes/mem-pointer-compression.md).
//!
//! The first chunk request reserves `RESERVE` bytes of address space with no access, and every arena chunk of every
//! thread arena and region is carved from it and committed on hand-out, so one `base()` turns any handle into an
//! address: `base() + (handle << UNIT_SHIFT)`. The first `GRANULE` is never handed out, so handle 0 is never an
//! object (the `Option<P<T>>` niche). Released chunks (region slabs) are decommitted and their ranges reused.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Handles count units of `1 << UNIT_SHIFT` bytes, so every `P` target is 8-aligned.
pub const UNIT_SHIFT: u32 = 3;
/// 2^32 units: 32 GiB.
pub const RESERVE: usize = 1 << (32 + UNIT_SHIFT);
/// Chunk sizes and offsets are multiples of this (a multiple of every supported page size).
const GRANULE: usize = 64 << 10;

/// Where the reservation lives: a fixed address, so `base()` is a constant and turning a handle into an address
/// costs an add, no load (a loaded base cost ~25% more instructions). 64 TiB + 4 GiB is clear of every placement seen
/// on the supported platforms: macOS keeps 4-448 GiB for the binary, the shared cache and its malloc zones and places
/// other mappings upward from 448 GiB; mimalloc hints from 2 to 30 TiB; Linux puts PIE binaries near 85 TiB and maps
/// top-down below the stack (near 128 TiB). It needs a 47-bit address space (x86-64, arm64 with 48-bit VA).
///
/// The value is chosen for arm64 codegen: one `movz` materializes it (a single 16-bit chunk, `0x4001 << 32`), and
/// bit 32 overlaps the offset range (`handle << 3` < 2^35), so LLVM cannot turn the add into an `orr`; `add x, base,
/// w, uxtw #3` then folds the zero-extension and the shift of a 32-bit handle into the one instruction. A base with
/// disjoint bits (0x4000_0000_0000) gave `mov w, w` + `orr` per dereference of a handle passed in a register.
pub const BASE_ADDR: usize = 0x4001_0000_0000;

static RESERVED: AtomicBool = AtomicBool::new(false);

/// The start of the reservation. A handle exists only after the first chunk, i.e. after the reservation.
#[inline]
pub fn base() -> *mut u8 {
    std::ptr::with_exposed_provenance_mut(BASE_ADDR)
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

static CHUNKS: Mutex<Chunks> = Mutex::new(Chunks { next: GRANULE, free: BTreeMap::new(), live: 0, peak: GRANULE });

#[cold]
fn fail(what: &str) -> ! {
    let live = CHUNKS.try_lock().map(|c| c.live).unwrap_or(0);
    eprintln!(
        "tsrs: {what} (compressed pointers: one {} GiB address range for every arena; {} MiB of it in use). \
         Build with the tsrs_core/plain-ptrs feature to lift the limit.",
        RESERVE >> 30,
        live >> 20
    );
    std::process::abort()
}

#[cfg(unix)]
fn reserve() {
    // SAFETY: a fresh anonymous mapping with no access at a hint; nothing else refers to it.
    let p = unsafe {
        libc::mmap(
            std::ptr::without_provenance_mut(BASE_ADDR),
            RESERVE,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE,
            -1,
            0,
        )
    };
    if p == libc::MAP_FAILED {
        fail("could not reserve the arena address range");
    }
    if p.addr() != BASE_ADDR {
        // SAFETY: the mapping just made, unused.
        unsafe { libc::munmap(p, RESERVE) };
        fail(&format!("the arena address range at {BASE_ADDR:#x} is taken (the system placed it at {:#x})", p.addr()));
    }
    // `base()` derives every arena pointer from the address.
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
    let base = base();
    let Some(off) = c.take(size) else {
        drop(c);
        fail("the arena address range is exhausted");
    };
    drop(c);
    // SAFETY: inside the reservation.
    let p = unsafe { base.add(off) };
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

/// How far into the reservation chunks have ever been carved (released ranges are reused before it grows).
pub fn reserved_high_water() -> usize {
    CHUNKS.lock().unwrap_or_else(|e| e.into_inner()).peak
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_ranges_are_reused_and_coalesced() {
        let mut c = Chunks { next: GRANULE, free: BTreeMap::new(), live: 0, peak: GRANULE };
        let a = c.take(GRANULE).unwrap();
        let b = c.take(3 * GRANULE).unwrap();
        let d = c.take(GRANULE).unwrap();
        assert_eq!((a, b, d), (GRANULE, 2 * GRANULE, 5 * GRANULE));
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
        assert!(p.addr() - base().addr() >= GRANULE);
    }
}

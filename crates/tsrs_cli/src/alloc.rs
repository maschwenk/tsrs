//! The global allocator: mimalloc. On Linux, mimalloc advises `MADV_HUGEPAGE` on its arenas, so every 2 MiB block
//! a thread has touched is resident in full. The blocks that are mostly untouched are those of mimalloc's large pages
//! (one 4 MiB page per size class per thread for objects of 85-512 KiB) and of partly used huge objects, so those
//! allocations come from a second mimalloc heap whose arena is advised `MADV_NOHUGEPAGE` (notes/mem-thp.md).
//! Small and medium objects keep the huge-page heap: they are packed densely and the TLB wants them on huge pages.

use std::alloc::{GlobalAlloc, Layout};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use libmimalloc_sys as mi;

pub struct Alloc;

/// The heap on 4 KiB pages; null until `init` (and on other systems): everything then goes to the default heap.
static COLD: AtomicPtr<mi::mi_heap_t> = AtomicPtr::new(null_mut());
static COLD_MIN: AtomicUsize = AtomicUsize::new(usize::MAX);
static COLD_MAX: AtomicUsize = AtomicUsize::new(usize::MAX);

#[inline]
fn cold(size: usize) -> *mut mi::mi_heap_t {
    // Relaxed: the three values are written once in `init`, before main spawns a thread (the spawn orders them); a
    // thread that read a stale value would only use the default heap, which is always correct.
    if size >= COLD_MIN.load(Ordering::Relaxed) && size <= COLD_MAX.load(Ordering::Relaxed) {
        COLD.load(Ordering::Relaxed)
    } else {
        null_mut()
    }
}

// SAFETY: every block comes from mimalloc (either heap), and `mi_free` / `mi_realloc` accept a block of any heap.
unsafe impl GlobalAlloc for Alloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let heap = cold(layout.size());
        // SAFETY: `heap` is null or the heap `init` created, which is never deleted.
        unsafe {
            if heap.is_null() {
                mi::mi_malloc_aligned(layout.size(), layout.align()).cast()
            } else {
                mi::mi_heap_malloc_aligned(heap, layout.size(), layout.align()).cast()
            }
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let heap = cold(layout.size());
        // SAFETY: as in `alloc`.
        unsafe {
            if heap.is_null() {
                mi::mi_zalloc_aligned(layout.size(), layout.align()).cast()
            } else {
                mi::mi_heap_zalloc_aligned(heap, layout.size(), layout.align()).cast()
            }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // SAFETY: `ptr` was returned by `alloc`, `alloc_zeroed` or `realloc` above.
        unsafe { mi::mi_free(ptr.cast::<c_void>()) }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let heap = cold(new_size);
        // SAFETY: `ptr` is a block of either heap; the heap argument only chooses where a moved block goes.
        unsafe {
            if heap.is_null() {
                mi::mi_realloc_aligned(ptr.cast::<c_void>(), new_size, layout.align()).cast()
            } else {
                mi::mi_heap_realloc_aligned(heap, ptr.cast::<c_void>(), new_size, layout.align()).cast()
            }
        }
    }
}

/// Creates the 4 KiB-page heap. Call first in main, before any thread is spawned.
pub fn init() {
    #[cfg(unix)]
    {
        // Probe knobs (bytes): TSRS_MI_COLD_MIN (0: off) and TSRS_MI_COLD_MAX.
        let var = |name: &str, default: usize| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
        let min = var("TSRS_MI_COLD_MIN", 0);
        let max = var("TSRS_MI_COLD_MAX", usize::MAX);
        if min == 0 {
            return;
        }
        // Address space only (MAP_NORESERVE); mimalloc falls back to OS allocations if it ever fills.
        const RESERVE: usize = 8 << 30;
        // SAFETY: a fresh private anonymous mapping, handed to mimalloc whole and never unmapped.
        unsafe {
            let p = libc::mmap(
                null_mut(),
                RESERVE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            );
            if p == libc::MAP_FAILED {
                return;
            }
            #[cfg(target_os = "linux")]
            libc::madvise(p, RESERVE, libc::MADV_NOHUGEPAGE);
            let mut arena: mi::mi_arena_id_t = null_mut();
            if !mi::mi_manage_os_memory_ex(p, RESERVE, true, false, true, -1, true, &mut arena) {
                return;
            }
            let heap = mi::mi_heap_new_in_arena(arena);
            if heap.is_null() {
                return;
            }
            COLD_MIN.store(min, Ordering::Relaxed);
            COLD_MAX.store(max, Ordering::Relaxed);
            COLD.store(heap, Ordering::Relaxed);
        }
    }
}

// The alloc-profile build installs tsrs_core's counting allocator (over mimalloc) instead; the arena-probe build
// its routing one.
#[cfg(not(any(feature = "alloc-profile", feature = "arena-probe")))]
#[global_allocator]
static GLOBAL: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

#[cfg(feature = "alloc-profile")]
mod census;
mod api;
mod lsp;

use tsrs_execute::{build, execute, sys, tsc};

// TSRS_MEM_SPLIT (tsrs_core::memsplit): mimalloc's view of its heap, every page of every thread.
#[cfg(not(feature = "alloc-profile"))]
fn mimalloc_heap_stats() -> tsrs_core::memsplit::HeapStats {
    #[repr(C)]
    struct HeapArea {
        blocks: *mut std::ffi::c_void,
        reserved: usize,
        committed: usize,
        used: usize,
        block_size: usize,
        full_block_size: usize,
        reserved1: *mut std::ffi::c_void,
    }
    unsafe extern "C" {
        fn mi_heap_visit_blocks(
            heap: *mut std::ffi::c_void,
            visit_blocks: bool,
            visitor: extern "C" fn(*const std::ffi::c_void, *const HeapArea, *mut std::ffi::c_void, usize, *mut std::ffi::c_void) -> bool,
            arg: *mut std::ffi::c_void,
        ) -> bool;
    }
    extern "C" fn visit(_heap: *const std::ffi::c_void, area: *const HeapArea, _block: *mut std::ffi::c_void, _size: usize, arg: *mut std::ffi::c_void) -> bool {
        // SAFETY: mimalloc passes a valid area and our `arg`, a `HeapStats`.
        let (area, stats) = unsafe { (&*area, &mut *arg.cast::<tsrs_core::memsplit::HeapStats>()) };
        stats.pages += 1;
        stats.capacity += area.committed;
        stats.used += area.used * area.full_block_size;
        // The page's header lies in front of its blocks, inside the same 4 KiB page.
        let start = area.blocks.addr() & !4095;
        stats.resident += tsrs_core::memsplit::resident(start, area.blocks.addr() + area.reserved - start);
        true
    }
    let mut stats = tsrs_core::memsplit::HeapStats::default();
    // SAFETY: a null heap is the main heap; the visitor only reads the areas. The callers' other threads are parked.
    unsafe { mi_heap_visit_blocks(std::ptr::null_mut(), false, visit, (&raw mut stats).cast()) };
    stats
}

#[cfg(not(feature = "alloc-profile"))]
fn mimalloc_collect(force: bool) {
    unsafe extern "C" {
        fn mi_collect(force: bool);
    }
    // SAFETY: collects the calling thread's heap; no arguments to get wrong.
    unsafe { mi_collect(force) };
}

// mimalloc-safe builds the `no_thp` feature as mimalloc's `allow_thp = 0`, and with that mimalloc turns transparent
// huge pages off for the whole process when it starts (`prctl(PR_SET_THP_DISABLE)` in `_mi_prim_mem_init`, run from a
// constructor before `main`), the arena's `MADV_HUGEPAGE` chunks included (`tsrs_core::reserve`). `no_thp` is only
// meant to keep mimalloc from advising its own heap, which `allow_thp = 0` still does; this turns THP back on for the
// rest (notes/mem-no-thp.md). An explicit MIMALLOC_ALLOW_THP keeps mimalloc's choice.
#[cfg(target_os = "linux")]
fn allow_transparent_huge_pages() {
    if std::env::var_os("MIMALLOC_ALLOW_THP").is_some() {
        return;
    }
    let zero: libc::c_ulong = 0;
    // SAFETY: PR_SET_THP_DISABLE takes integer arguments only and changes nothing but this process's THP flag.
    unsafe { libc::prctl(libc::PR_SET_THP_DISABLE, zero, zero, zero, zero) };
}

fn main() {
    #[cfg(target_os = "linux")]
    allow_transparent_huge_pages();
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(not(feature = "alloc-profile"))]
    tsrs_core::memsplit::set_heap_hooks(mimalloc_heap_stats, mimalloc_collect);
    #[cfg(feature = "alloc-profile")]
    tsrs_execute::set_census_hook(census::run);
    // main.go:21: `--lsp` runs the language server (its threads have their own stacks); `--api` runs the
    // native API server (docs/NODE_API.md).
    if args.first().map(String::as_str) == Some("--lsp") {
        std::process::exit(lsp::run_lsp(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("--api") {
        // Checking recurses deeply; serve requests on a large stack like the command line below.
        let rest = args[1..].to_vec();
        let status = std::thread::Builder::new().stack_size(512 << 20).spawn(move || api::run_api(&rest)).unwrap().join().unwrap_or(1);
        std::process::exit(status);
    }
    // Type checking recurses deeply; run on a thread with a large stack
    // (Go's goroutine stacks grow on demand).
    let status = std::thread::Builder::new()
        .name("tsrs".to_string()) // the alloc profile's "main" thread group
        .stack_size(512 << 20)
        .spawn(move || {
            let sys: &'static sys::osSys = Box::leak(Box::new(sys::new_system()));
            let result = execute::command_line(sys, args);
            tsc::System::flush(sys);
            tsrs_core::alloc_profile_dump();
            tsrs_core::sitecount::dump();
            result.status
        })
        .unwrap()
        .join()
        .unwrap_or(tsc::ExitStatus::NotImplemented);
    // TSRS_UNION_CACHE_STATS / TSRS_UNION_CACHE=shadow: the union front cache's totals.
    #[cfg(feature = "checker")]
    tsrs_compiler::Checker::union_cache_finish();
    // TSRS_INFER_MEMO_STATS / TSRS_INFER_MEMO=shadow: the inference memo's totals.
    #[cfg(feature = "checker")]
    tsrs_compiler::Checker::infer_memo_finish();
    tsrs_core::memsplit::report("exit");
    tsrs_core::arena_probe::dump();
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let _ = std::io::Write::flush(&mut std::io::stderr());
    std::process::exit(status as i32)
}

// The alloc-profile build installs tsrs_core's counting allocator (over mimalloc) instead.
#[cfg(not(feature = "alloc-profile"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
    extern "C" {
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
    extern "C" {
        fn mi_collect(force: bool);
    }
    // SAFETY: collects the calling thread's heap; no arguments to get wrong.
    unsafe { mi_collect(force) };
}

fn main() {
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
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let _ = std::io::Write::flush(&mut std::io::stderr());
    std::process::exit(status as i32)
}

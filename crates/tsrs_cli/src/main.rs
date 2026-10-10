// The alloc-profile build installs tsrs_core's counting allocator (over mimalloc) instead.
#[cfg(not(feature = "alloc-profile"))]
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

// Keeps transparent huge pages on for the process, which the arena's `MADV_HUGEPAGE` chunks need
// (`tsrs_core::reserve`): `PR_SET_THP_DISABLE` is inherited from the parent and survives `exec`, and mimalloc sets it
// at start-up when `allow_thp` is 0 there (notes/mem-no-thp.md). An explicit MIMALLOC_ALLOW_THP keeps mimalloc's choice.
#[cfg(target_os = "linux")]
fn allow_transparent_huge_pages() {
    // SAFETY: a NUL-terminated name; only this thread runs, so nothing writes the environment meanwhile.
    if !unsafe { libc::getenv(c"MIMALLOC_ALLOW_THP".as_ptr()) }.is_null() {
        return;
    }
    let zero: libc::c_ulong = 0;
    // SAFETY: PR_SET_THP_DISABLE takes integer arguments only and changes nothing but this process's THP flag.
    unsafe { libc::prctl(libc::PR_SET_THP_DISABLE, zero, zero, zero, zero) };
}

// Huge pages for the mimalloc heap where memory is plentiful (notes/perf-heap-thp-by-memory.md). With `allow_thp` on,
// mimalloc advises every arena it reserves with `MADV_HUGEPAGE`, and every partly filled thread-local page then keeps
// a whole 2 MiB page resident: fewer page faults and TLB misses for 25-35% more peak RSS (notes/mem-no-thp.md). The
// advice is on when the memory available to the process (cgroup limits included) is at least `MIN_AVAILABLE`, off
// otherwise; `TSRS_HEAP_THP=0|1` forces it and an explicit MIMALLOC_ALLOW_THP overrides both.
//
// mimalloc advises an arena once, when an allocation reserves it, and the Rust runtime allocates before `main` (the
// first 1 GiB arena is reserved there). So `configure` runs from `.init_array`, after mimalloc's own constructor and
// before the runtime starts, and allocates nothing.
#[cfg(target_os = "linux")]
mod heap_thp {
    use std::ffi::{c_int, c_long};
    use std::sync::OnceLock;

    // `mi_option_allow_thp` in mimalloc 3.5's `mi_option_t`. libmimalloc-sys2's `mi_option_allow_thp` is 37, its
    // number in mimalloc 2, which is `mi_option_page_max_candidates` in 3.x; the test below checks this one.
    pub(crate) const MI_OPTION_ALLOW_THP: c_int = 43;
    // Four times the largest peak a bench project reaches with the advice on (notes/perf-heap-thp-by-memory.md).
    const MIN_AVAILABLE: usize = 20 << 30;

    unsafe extern "C" {
        pub(crate) fn mi_option_get(option: c_int) -> c_long;
        fn mi_option_set_default(option: c_int, value: c_long);
    }

    struct Decision {
        on: bool,
        available: Option<usize>,
    }

    static DECISION: OnceLock<Decision> = OnceLock::new();

    // Not in the unit-test binary, which checks mimalloc's compiled default.
    #[cfg(not(test))]
    #[used]
    #[unsafe(link_section = ".init_array")]
    static CONFIGURE: extern "C" fn() = configure;

    extern "C" fn configure() {
        super::allow_transparent_huge_pages();
        // SAFETY: a NUL-terminated name; only this thread runs, so nothing writes the environment meanwhile.
        let force = unsafe { libc::getenv(c"TSRS_HEAP_THP".as_ptr()) };
        // SAFETY: getenv returned a NUL-terminated string that stays valid while the environment is not written.
        let force = (!force.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(force) }.to_bytes());
        let available = tsrs_core::memsplit::available_memory();
        let on = match force {
            Some(b"0") => false,
            Some(b"1") => true,
            _ => available.is_some_and(|a| a >= MIN_AVAILABLE),
        };
        // SAFETY: integer arguments only. `set_default` leaves the option alone when MIMALLOC_ALLOW_THP set it.
        unsafe { mi_option_set_default(MI_OPTION_ALLOW_THP, c_long::from(on)) };
        // SAFETY: integer argument only.
        let on = unsafe { mi_option_get(MI_OPTION_ALLOW_THP) } != 0;
        let _ = DECISION.set(Decision { on, available });
    }

    // The decision as rows of the tsrs-only `--extendedDiagnostics` table.
    pub(crate) fn record() {
        if let Some(decision) = DECISION.get() {
            tsrs_core::phases::count("Heap huge pages", u64::from(decision.on));
            if let Some(available) = decision.available {
                tsrs_core::phases::count("Available memory (MiB)", (available >> 20) as u64);
            }
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    heap_thp::record();
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    // The option number `heap_thp` sets must be mimalloc's `allow_thp`: with a wrong one (a mimalloc upgrade that
    // renumbers `mi_option_t`), the heap's huge-page advice would ignore the memory rule and TSRS_HEAP_THP and some
    // other allocator option would change instead. Nothing in the test process sets the option, so it reads
    // mimalloc's compiled default for Linux, 2 (CMake `MI_ALLOW_THP=FULL`).
    #[test]
    fn option_number_is_allow_thp() {
        if std::env::var_os("MIMALLOC_ALLOW_THP").is_some() {
            return;
        }
        // SAFETY: integer argument only.
        assert_eq!(unsafe { super::heap_thp::mi_option_get(super::heap_thp::MI_OPTION_ALLOW_THP) }, 2);
    }
}

// The alloc-profile build installs tsrs_core's counting allocator (over mimalloc) instead.
#[cfg(not(feature = "alloc-profile"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "alloc-profile")]
mod census;
mod api;
mod build;
mod execute;
mod lsp;
mod sys;
mod tsc;
#[cfg(test)]
mod tsctests;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
    // (Go's goroutine stacks grow on demand). The thread ends the process itself once the output is flushed.
    let _ = std::thread::Builder::new()
        .name("tsrs".to_string()) // the alloc profile's "main" thread group
        .stack_size(512 << 20)
        .spawn(move || {
            let sys: &'static sys::osSys = Box::leak(Box::new(sys::new_system()));
            let result = execute::command_line(sys, args);
            tsc::System::flush(sys);
            tsrs_core::alloc_profile_dump();
            tsrs_core::sitecount::dump();
            finish(result.status)
        })
        .unwrap()
        .join();
    // Only a panic on the tsrs thread gets here.
    finish(tsc::ExitStatus::NotImplemented)
}

// The end of a command-line run: the env-gated checker reports, then the exit. tsrs-only: `_exit` instead of
// `exit` once the output is written. `exit` runs mimalloc's process-done handler, which collects the heap and
// returns every free range to the OS (vscode, 64 vCPUs: ~300 `madvise` calls over ~600 MiB, 5-8 ms), and the
// thread's own exit returns its touched stack pages (~2 ms); the kernel frees all of it at the exit anyway.
fn finish(status: tsc::ExitStatus) -> ! {
    use std::io::Write;
    let mut code = status as i32;
    // TSRS_UNION_CACHE_STATS / TSRS_UNION_CACHE=shadow: the union front cache's totals.
    #[cfg(feature = "checker")]
    tsrs_compiler::Checker::union_cache_finish();
    // TSRS_INFER_MEMO_STATS / TSRS_INFER_MEMO=shadow: the inference memo's totals.
    #[cfg(feature = "checker")]
    tsrs_compiler::Checker::infer_memo_finish();
    // TSRS_DERIVED_VARIANCE=shadow reports each disagreement as it is found and fails the run at the end.
    #[cfg(feature = "checker")]
    if tsrs_compiler::Checker::derived_variance_finish() > 0 {
        code = 7;
    }
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // mimalloc prints its statistics from the process-done handler, and an instrumented (PGO training) binary writes
    // its profile from an exit handler too: `LLVM_PROFILE_FILE` is set only by .github/scripts/pgo-train.sh, and
    // `_exit` would leave the profile at 0 bytes (which it did between #143 and this check).
    #[cfg(unix)]
    if std::env::var_os("MIMALLOC_SHOW_STATS").is_none()
        && std::env::var_os("MIMALLOC_VERBOSE").is_none()
        && std::env::var_os("LLVM_PROFILE_FILE").is_none()
    {
        // SAFETY: the output is flushed and nothing else in the process needs to run before it ends.
        unsafe { libc::_exit(code) }
    }
    std::process::exit(code)
}

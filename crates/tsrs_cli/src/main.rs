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
    // TSRS_DERIVED_VARIANCE=shadow reports each disagreement as it is found and fails the run at the end.
    #[cfg(feature = "checker")]
    if tsrs_compiler::Checker::derived_variance_finish() > 0 {
        std::process::exit(7);
    }
    std::process::exit(status as i32);
}
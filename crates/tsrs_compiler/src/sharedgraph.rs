//! tsrs-only: a type graph shared between the checker threads of the type-check pass (notes/spike-shared-graph.md).
//!
//! One seed checker checks a sample of the program's files on a thread of its own; its region is then frozen
//! (`tsrs_core::sharedgraph::freeze`) and every checker of the pool becomes a fork of it (`Checker::fork`): a fork reads
//! the seed's types, symbols and links and keeps what it would write into them in its own overlay. With `--maxMemory`,
//! a checker the pass retires is replaced by a fresh fork, which starts with the seed's graph instead of rebuilding it,
//! and the pool does not wait for the seed: its checkers start as plain checkers and are retired for forks at their
//! first file boundary after the freeze.
//!
//! Built with `--features shared-graph`, on with `TSRS_SHARED_GRAPH=1`. `TSRS_SHARED_GRAPH_SEED_PERCENT` sets the
//! seed's share of the estimated checking work, in percent (default 5 with `--maxMemory`, 1 without;
//! notes/spike-shared-graph.md 10.2).

use std::sync::OnceLock;

use tsrs_ast::SourceFile;
use tsrs_core::P;

use crate::program::Program;

/// Whether the shared graph is on (`TSRS_SHARED_GRAPH=1` in a build with the feature).
pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("TSRS_SHARED_GRAPH").as_deref() {
        Ok("1" | "on") if tsrs_core::sharedgraph::COMPILED_IN => true,
        Ok("1" | "on") => {
            eprintln!("tsrs: TSRS_SHARED_GRAPH=1 needs a build with --features shared-graph; running without it");
            false
        }
        _ => false,
    })
}

// The seed's share of the estimated checking work (the files' weights), in parts per million ("2.5" percent ->
// 25000). Under `--maxMemory` the pool does not wait for the seed, so a larger one costs no serial time: 5% measured
// best there (-1.4% instructions, -3% wall against 1% on the 38k-file codebase; 10% and 20% lost both). Without a
// target every checker waits for the seed, whose time grows with its size, so the default stays at the first round's 1%.
fn seed_ppm(recycle: bool) -> u64 {
    let default = if recycle { 5.0 } else { 1.0 };
    let percent: f64 = std::env::var("TSRS_SHARED_GRAPH_SEED_PERCENT").ok().and_then(|v| v.trim().trim_end_matches('%').parse().ok()).unwrap_or(default);
    (percent * 10_000.0).round().clamp(0.0, 1_000_000.0) as u64
}

/// The seed files, as positions in `files`, in program order: the lighter half of the checked files, evenly spaced up
/// to `ppm` (parts per million) of the checked weight. Never a declaration file, and never a file that may be freed
/// once it is checked (one with a region of its own, `fileregions`), since a frozen object must not point into a freed
/// tree.
pub(crate) fn seed_positions(program: &Program, files: &[P<SourceFile>], weight: &dyn Fn(u32) -> u64, ppm: u64) -> Vec<u32> {
    let eligible = |i: usize| {
        let f = files[i];
        !f.is_declaration_file() && !f.is_check_leaf() && !crate::fileregions::has_region(f) && !program.skip_type_checking(f, false) && weight(i as u32) > 0
    };
    let total: u64 = (0..files.len()).map(|i| weight(i as u32)).sum();
    let budget = (total as u128 * ppm as u128 / 1_000_000) as u64;
    let mut candidates: Vec<usize> = (0..files.len()).filter(|&i| eligible(i)).collect();
    if candidates.is_empty() || budget == 0 {
        return Vec::new();
    }
    // The lighter half, back in program order.
    candidates.sort_by_key(|&i| (weight(i as u32), i));
    candidates.truncate(candidates.len().div_ceil(2));
    candidates.sort_unstable();
    let light: u64 = candidates.iter().map(|&i| weight(i as u32)).sum();
    let step = (light / budget.max(1)).max(1) as usize;
    let mut out = Vec::new();
    let mut sum = 0;
    for &i in candidates.iter().step_by(step) {
        if sum >= budget {
            break;
        }
        sum += weight(i as u32);
        out.push(i as u32);
    }
    out
}

/// Makes the checker's overlay the current thread's (`tsrs_core::sharedgraph::enter_overlay`), with the shared graph on.
#[inline]
pub(crate) fn enter_checker(c: &crate::checkerpool::Checker) {
    #[cfg(feature = "checker")]
    if enabled() {
        tsrs_core::sharedgraph::enter_overlay(&c.overlay);
    }
    #[cfg(not(feature = "checker"))]
    let _ = c;
}

/// The frozen seed checker.
#[cfg(feature = "checker")]
#[derive(Clone, Copy)]
pub(crate) struct Base(pub &'static tsrs_checker::Checker);

// SAFETY: the base is frozen: no thread writes it after `seed` returns, and forks only read it.
#[cfg(feature = "checker")]
unsafe impl Send for Base {}
// SAFETY: as for `Send`.
#[cfg(feature = "checker")]
unsafe impl Sync for Base {}

/// What the seed thread hands over: the seed checker, its region's chunks (to freeze) and when it started.
#[cfg(feature = "checker")]
struct SeedOut(&'static tsrs_checker::Checker, Vec<(usize, usize)>, std::time::Instant);
// SAFETY: the checker is handed from the seed thread, which ends, to the thread that freezes it; nothing else refers
// to it until then.
#[cfg(feature = "checker")]
unsafe impl Send for SeedOut {}

#[cfg(feature = "checker")]
static SEED_THREAD: std::sync::Mutex<Option<std::thread::JoinHandle<SeedOut>>> = std::sync::Mutex::new(None);
#[cfg(feature = "checker")]
static BASE: OnceLock<Base> = OnceLock::new();
/// `type_count` of a fresh checker: a pool checker with more was used before the pass and is not replaced.
#[cfg(feature = "checker")]
static FRESH_TYPES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);

#[cfg(feature = "checker")]
pub(crate) fn note_fresh_checker(c: &crate::checkerpool::Checker) {
    // Relaxed: written before the pass's threads are spawned.
    FRESH_TYPES.store(c.type_count, std::sync::atomic::Ordering::Relaxed);
}

/// Starts checking the seed files on a thread of its own (at the start of the type-check pass, once the leaves are
/// classified, so that the leaf guard also keeps the seed from reading another leaf).
#[cfg(feature = "checker")]
pub(crate) fn start_seed(program: &'static Program, weights: Vec<i64>, recycle: bool) {
    let start = std::time::Instant::now();
    let handle = std::thread::Builder::new()
        .name("checker-seed".into())
        .stack_size(crate::checkerpool::CHECKER_STACK_SIZE)
        .spawn(move || {
            let files = &program.files;
            let positions = seed_positions(program, files, &|i: u32| weights.get(i as usize).copied().unwrap_or(0).max(0) as u64, seed_ppm(recycle));
            tsrs_ast::use_id_blocks();
            tsrs_core::sharedgraph::set_seed_thread(true);
            // Everything the seed checker allocates goes to this region, which is frozen afterwards. Not a scratch scope:
            // what would escape one (lazily filled data of shared objects) must be frozen too, or the forks would share
            // it unprotected.
            let region = tsrs_core::arena::Region::new_scratch(32 << 20);
            let scope = region.enter();
            let mut c = tsrs_checker::new_checker(program);
            c.seed_mode = true;
            tsrs_core::sharedgraph::enter_overlay(&c.overlay);
            let ctx = tsrs_checker::Context::background();
            for &i in &positions {
                let _ = c.get_diagnostics_exported(&ctx, files[i as usize]);
            }
            c.assert_freezable();
            c.seed_mode = false;
            drop(scope);
            let chunks = region.chunks();
            // Never freed: the forks read it for the rest of the process.
            #[expect(clippy::mem_forget, reason = "the frozen seed region must outlive every fork, to the end of the process")]
            std::mem::forget(region);
            SeedOut(Box::leak(c), chunks, start)
        })
        .expect("failed to spawn the seed checker thread");
    *SEED_THREAD.lock().unwrap() = Some(handle);
}

/// The frozen seed: the first caller joins the seed thread and freezes its region; the others wait for it.
#[cfg(feature = "checker")]
fn wait_base() -> Base {
    *BASE.get_or_init(|| {
        let handle = SEED_THREAD.lock().unwrap().take().expect("shared graph: the seed was not started");
        let out = handle.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
        tsrs_core::sharedgraph::freeze(&out.1);
        if debug_regions() {
            let chunks: Vec<String> = out.1.iter().map(|&(s, n)| format!("{s:x}+{n:x}")).collect();
            eprintln!("dbg-seed chunks={}", chunks.join(","));
        }
        tsrs_core::phases::record("Checkers: seed", out.2.elapsed());
        Base(out.0)
    })
}

/// The frozen seed if it is ready, without waiting (freezes it if the seed thread has finished).
#[cfg(feature = "checker")]
pub(crate) fn try_base() -> Option<Base> {
    if !enabled() {
        return None;
    }
    if let Some(b) = BASE.get() {
        return Some(*b);
    }
    let finished = SEED_THREAD.lock().unwrap().as_ref().is_some_and(std::thread::JoinHandle::is_finished);
    finished.then(wait_base)
}

/// At the start of the type-check pass without `--maxMemory`: replaces an unused plain pool checker by a fork of the
/// frozen seed, waiting for it.
#[cfg(feature = "checker")]
pub(crate) fn fork_into(slot: &mut Box<crate::checkerpool::Checker>) {
    // Relaxed: written by create_checkers before the pass's threads were spawned.
    if slot.is_fork || slot.type_count != FRESH_TYPES.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    *slot = fork_of(wait_base());
}

/// A fork of the frozen seed if it is ready: the replacement for a checker `--maxMemory` retires.
#[cfg(feature = "checker")]
pub(crate) fn fresh_fork() -> Option<Box<crate::checkerpool::Checker>> {
    try_base().map(fork_of)
}

/// `Checker::fork`, leaving the fork's own overlay current (the thread's may still be that of a retired checker).
#[cfg(feature = "checker")]
fn fork_of(base: Base) -> Box<crate::checkerpool::Checker> {
    let c = tsrs_checker::Checker::fork(base.0);
    tsrs_core::sharedgraph::enter_overlay(&c.overlay);
    c
}

/// Debugging: `TSRS_DEBUG_REGIONS=1` logs the seed's chunks and every retired checker region's chunks, and retires
/// those regions for good (pages given back, addresses never reused), so that a stale pointer into a retired checker
/// faults at an address that names its region (notes/spike-shared-graph.md section 10.1).
pub(crate) fn debug_regions() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| std::env::var_os("TSRS_DEBUG_REGIONS").is_some())
}

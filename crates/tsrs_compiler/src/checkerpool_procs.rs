// Process mode for the checker pool (TSRS_CHECKER_PROCESSES=<n>, notes/perf-checker-processes.md). One checker, the
// base, checks a warm-up set of files; then the process forks and every child continues the same checker on its own
// slice of the remaining files, so what the base built (types, symbols, signatures, relations) is shared copy-on-write
// instead of being rebuilt by every checker. Each child sends its files' diagnostics back over a pipe and exits.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use rustc_hash::FxHashMap;
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::P;

use crate::checkerpool::{checkerPool, thread_cpu_seconds, Checker};
use crate::program::Program;

// Set by the command line for a plain one-program compile; every other host (build mode, the test runners, the
// language server, the API) runs several programs or requests on threads, where forking is not safe.
static ALLOWED: AtomicBool = AtomicBool::new(false);

/// The command line's one-program compile may use checker processes (when TSRS_CHECKER_PROCESSES asks for them).
pub fn allow_checker_processes() {
    // Set once before the program is created, on the thread that creates it.
    ALLOWED.store(true, Ordering::Relaxed);
}

// TSRS_CHECKER_PROCESSES=<n>: the number of worker processes (n >= 2), 0 or unset for checker threads.
fn requested_processes() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::env::var("TSRS_CHECKER_PROCESSES").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(0))
}

/// The number of checker processes for `program`, or 0 for checker threads.
pub(crate) fn checker_processes_for(program: &Program) -> usize {
    // Checked once per program, before any checker exists.
    if !cfg!(unix) || !ALLOWED.load(Ordering::Relaxed) {
        return 0;
    }
    let n = requested_processes();
    let options = program.options();
    if n < 2 || !options.no_emit.is_true() || program.single_threaded() || options.checkers.is_some() {
        return 0;
    }
    n
}

// TSRS_CHECKER_PROCESSES_PAUSE=<dir> (measurement): when a process has checked its slice it creates `<dir>/<name>`;
// children then wait for `<dir>/go`, so the physical memory of the whole process tree can be taken at its peak.
fn pause_point(name: &str, wait: bool) {
    let Ok(dir) = std::env::var("TSRS_CHECKER_PROCESSES_PAUSE") else { return };
    let _ = std::fs::write(format!("{dir}/{name}"), std::process::id().to_string());
    while wait && !std::path::Path::new(&format!("{dir}/go")).exists() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn stats_enabled() -> bool {
    std::env::var("TSRS_CHECKER_PROCESSES_STATS").is_ok_and(|v| !v.is_empty() && v != "0")
}

// What the children reported, kept for the global diagnostics and counters asked for after the check.
#[derive(Default)]
pub(crate) struct ProcessResults {
    pub(crate) globals: Vec<P<Diagnostic>>,
    // Types, symbols and instantiations the children created after the fork (summed).
    pub(crate) counts: (u64, u64, u64),
}

pub(crate) struct ProcessState {
    // Slice per program file index (the locality assignment for `processes` checkers).
    pub(crate) slices: Vec<usize>,
    pub(crate) processes: usize,
    pub(crate) forked: AtomicBool,
    pub(crate) results: Mutex<ProcessResults>,
}

impl ProcessState {
    pub(crate) fn new(slices: Vec<usize>, processes: usize) -> ProcessState {
        ProcessState { slices, processes, forked: AtomicBool::new(false), results: Mutex::new(ProcessResults::default()) }
    }
}

// TSRS_CHECKER_WARMUP=<strategy> (measurement): which files the base checks before the fork.
//   none | prefix:<permille> (program-order prefix by checked weight) | stride:<permille> (every k-th checked file of
//   each slice, checked hub-first) | stride-po:<permille> (the same, checked in program order) | indeg:<count> (the
//   most imported files, hub-first) | files:<path> (file names, one per line, checked in program order)
fn warmup_files(program: &Program, files: &[P<SourceFile>], checked: &[bool], weights: &[i64], slices: &[usize], processes: usize) -> Vec<usize> {
    let spec = std::env::var("TSRS_CHECKER_WARMUP").unwrap_or_else(|_| "prefix:20".to_string());
    let (kind, arg) = spec.split_once(':').unwrap_or((spec.as_str(), ""));
    let permille = arg.parse::<i64>().unwrap_or(0);
    let in_degrees = || -> Vec<usize> {
        let mut in_degree = vec![0usize; files.len()];
        for targets in crate::checkerpool::get_import_targets(program) {
            for t in targets {
                in_degree[t] += 1;
            }
        }
        in_degree
    };
    match kind {
        "none" => Vec::new(),
        "prefix" => {
            let total: i64 = weights.iter().sum();
            let goal = total * permille / 1000;
            let mut sum = 0;
            let mut out = Vec::new();
            for (i, &w) in weights.iter().enumerate() {
                if sum >= goal {
                    break;
                }
                if checked[i] {
                    sum += w;
                    out.push(i);
                }
            }
            out
        }
        "stride" | "stride-po" => {
            let step = (1000 / permille.max(1)).max(1) as usize;
            let mut out = Vec::new();
            for s in 0..processes {
                out.extend((0..files.len()).filter(|&i| checked[i] && slices[i] == s).step_by(step));
            }
            if kind == "stride" {
                let in_degree = in_degrees();
                out.sort_by(|&a, &b| in_degree[b].cmp(&in_degree[a]).then(a.cmp(&b)));
            } else {
                out.sort_unstable();
            }
            out
        }
        "indeg" => {
            let in_degree = in_degrees();
            let mut out: Vec<usize> = (0..files.len()).filter(|&i| checked[i]).collect();
            out.sort_by(|&a, &b| in_degree[b].cmp(&in_degree[a]).then(a.cmp(&b)));
            out.truncate(permille as usize);
            out
        }
        "files" => {
            let text = std::fs::read_to_string(arg).expect("TSRS_CHECKER_WARMUP files list");
            let wanted: rustc_hash::FxHashSet<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            (0..files.len()).filter(|&i| checked[i] && wanted.contains(files[i].file_name())).collect()
        }
        _ => panic!("unknown TSRS_CHECKER_WARMUP {spec:?}"),
    }
}

// Gives every node and binder symbol of the program its id before the fork, so the children do not write ids into
// shared AST and binder pages (lazy ids were 70% of the arena pages children copied; notes/perf-checker-processes.md).
// Deterministic: each file gets a range of node ids and of symbol ids (prefix sums of a counting pass) and assigns
// them in tree order; a symbol belongs to the file of its first declaration. Runs on the worker pool, which is idle
// again when this returns.
fn preassign_ids(program: &Program, files: &[P<SourceFile>]) -> (u64, u64) {
    use rayon::prelude::*;
    fn owned(symbol: P<tsrs_ast::Symbol>, file: P<SourceFile>) -> bool {
        tsrs_ast::symbol_id_unset(symbol)
            && symbol.declarations().first().is_some_and(|&d| tsrs_ast::get_source_file_of_node(d) == Some(file))
    }
    // Visits the nodes of `file` in tree order with the symbols each one introduces.
    fn walk(node: P<tsrs_ast::Node>, file: P<SourceFile>, on_node: &mut dyn FnMut(P<tsrs_ast::Node>), on_symbol: &mut dyn FnMut(P<tsrs_ast::Symbol>)) {
        on_node(node);
        if let Some(symbol) = node.symbol().filter(|&s| owned(s, file)) {
            on_symbol(symbol);
        }
        if let Some(symbol) = node.local_symbol().filter(|&s| owned(s, file)) {
            on_symbol(symbol);
        }
        if let Some(locals) = node.locals() {
            for symbol in locals.values() {
                if owned(symbol, file) {
                    on_symbol(symbol);
                }
            }
        }
        node.for_each_child(&mut |child| {
            walk(child, file, on_node, on_symbol);
            false
        });
    }
    let parallel = |f: &(dyn Fn(usize) -> (u64, u64) + Sync)| -> Vec<(u64, u64)> {
        if program.single_threaded() {
            (0..files.len()).map(f).collect()
        } else {
            crate::program::worker_pool().install(|| (0..files.len()).into_par_iter().map(f).collect())
        }
    };
    let counts = parallel(&|i| {
        let file = files[i];
        let mut nodes = 0u64;
        let mut seen: rustc_hash::FxHashSet<P<tsrs_ast::Symbol>> = rustc_hash::FxHashSet::default();
        walk(file.as_node(), file, &mut |n| nodes += u64::from(tsrs_ast::node_id_unset(n)), &mut |s| {
            seen.insert(s);
        });
        (nodes, seen.len() as u64)
    });
    let (total_nodes, total_symbols) = counts.iter().fold((0, 0), |a, c| (a.0 + c.0, a.1 + c.1));
    let mut node_next = tsrs_ast::reserve_node_ids(total_nodes);
    let mut symbol_next = tsrs_ast::reserve_symbol_ids(total_symbols);
    let mut starts = Vec::with_capacity(files.len());
    for &(nodes, symbols) in &counts {
        starts.push((node_next, symbol_next));
        node_next += nodes;
        symbol_next += symbols;
    }
    parallel(&|i| {
        let file = files[i];
        let (mut node_id, mut symbol_id) = starts[i];
        walk(
            file.as_node(),
            file,
            &mut |n| {
                if tsrs_ast::set_node_id_if_unset(n, node_id) {
                    node_id += 1;
                }
            },
            &mut |s| {
                if tsrs_ast::set_symbol_id_if_unset(s, symbol_id) {
                    symbol_id += 1;
                }
            },
        );
        debug_assert_eq!((node_id, symbol_id), (starts[i].0 + counts[i].0, starts[i].1 + counts[i].1));
        (0, 0)
    });
    (total_nodes, total_symbols)
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn get_u32(input: &mut &[u8]) -> u32 {
    let (head, rest) = input.split_at(4);
    *input = rest;
    u32::from_le_bytes(head.try_into().unwrap())
}

fn get_u64(input: &mut &[u8]) -> u64 {
    let (head, rest) = input.split_at(8);
    *input = rest;
    u64::from_le_bytes(head.try_into().unwrap())
}

fn counts_of(c: &Checker) -> (u64, u64, u64) {
    (c.type_count as u64, c.symbol_count as u64, c.total_instantiation_count as u64)
}

impl checkerPool {
    // Checks `files` (the program's files) with the base checker and forked children; returns each file's
    // diagnostics as `collect` produced them, by index into `files`.
    #[cfg(unix)]
    pub(crate) fn check_in_processes(
        &self,
        files: &[P<SourceFile>],
        collect: &(dyn Fn(&mut Checker, P<SourceFile>) -> Vec<P<Diagnostic>> + Sync),
    ) -> Vec<Vec<P<Diagnostic>>> {
        let program = self.program;
        let state = self.state();
        let procs = state.processes.as_ref().expect("process mode");
        assert!(!procs.forked.swap(true, Ordering::Relaxed), "the checker processes run once per program");
        let stats = stats_enabled();
        let start = std::time::Instant::now();
        let mut guard = state.checkers[0].0.lock().unwrap();
        let hw_before_warmup = tsrs_core::ptr::reserve_stats().map_or(0, |s| s.1);

        let checked: Vec<bool> = files.iter().map(|&f| !program.skip_type_checking(f, false)).collect();
        let weights = crate::checkerpool::checked_file_weights(program);
        let slices = &procs.slices;
        let n = procs.processes;
        let warmup = warmup_files(program, files, &checked, &weights, slices, n);
        let mut in_warmup = vec![false; files.len()];
        for &i in &warmup {
            in_warmup[i] = true;
        }

        let mut results: Vec<Vec<P<Diagnostic>>> = vec![Vec::new(); files.len()];
        let warm_cpu = thread_cpu_seconds();
        // TSRS_CHECKER_WARMUP_EXPORTS=<count> (measurement): before the warm-up files, resolve the exports of the most
        // imported modules (types only, no file is checked).
        if let Some(k) = std::env::var("TSRS_CHECKER_WARMUP_EXPORTS").ok().and_then(|v| v.parse::<usize>().ok()) {
            let mut in_degree = vec![0usize; files.len()];
            for targets in crate::checkerpool::get_import_targets(program) {
                for t in targets {
                    in_degree[t] += 1;
                }
            }
            let mut order: Vec<usize> = (0..files.len()).collect();
            order.sort_by(|&a, &b| in_degree[b].cmp(&in_degree[a]).then(a.cmp(&b)));
            for &i in order.iter().take(k) {
                guard.warm_up_module_exports(files[i]);
            }
        }
        for &i in &warmup {
            results[i] = collect(&mut guard, files[i]);
        }
        let warm_cpu = thread_cpu_seconds() - warm_cpu;
        let warm_wall = start.elapsed().as_secs_f64();
        if std::env::var("TSRS_CHECKER_PREASSIGN_IDS").is_ok() {
            let t = std::time::Instant::now();
            let (nodes, symbols) = preassign_ids(program, files);
            if stats {
                eprintln!("procs\tpre-assigned ids: {nodes} nodes, {symbols} symbols in {:.3}s", t.elapsed().as_secs_f64());
            }
        }
        let hw_at_fork = tsrs_core::ptr::reserve_stats().map_or(0, |s| s.1);
        if stats {
            eprintln!("procs\tarena high water: before warm-up {:#x}, at fork {:#x}", hw_before_warmup, hw_at_fork);
        }
        let base_counts = counts_of(&guard);
        let file_index: FxHashMap<P<SourceFile>, u32> = files.iter().enumerate().map(|(i, &f)| (f, i as u32)).collect();

        // Child `k` checks slice `k`; the parent checks slice 0 itself unless TSRS_CHECKER_PARENT_IDLE is set.
        let parent_checks = std::env::var("TSRS_CHECKER_PARENT_IDLE").is_err();
        let first_child = usize::from(parent_checks);
        let write_trace = std::env::var("TSRS_CHECKER_PROCESSES_WRITETRACE").ok();
        let mut work = |k: usize| -> Vec<u8> {
            let slice = first_child + k;
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            if write_trace.is_some() {
                let base = 0x4001_0000_0000usize;
                if std::env::var("TSRS_CHECKER_PROCESSES_WRITETRACE_HEAP").is_ok() {
                    tsrs_core::procs::writetrace::add_tagged_regions(100);
                }
                // SAFETY: the child is single-threaded; the range is the arena handed out before the fork.
                unsafe { tsrs_core::procs::writetrace::start(base + (64 << 10), base + hw_at_fork) };
            }
            let child_start = std::time::Instant::now();
            let mut checked_files = 0u32;
            for i in 0..files.len() {
                if slices[i] == slice && !in_warmup[i] {
                    results[i] = collect(&mut guard, files[i]);
                    checked_files += 1;
                }
            }
            let globals = guard.get_global_diagnostics();
            let wall = child_start.elapsed().as_secs_f64();
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            if let Some(dir) = &write_trace {
                tsrs_core::procs::writetrace::dump(&format!("{dir}/writes-{slice}.txt"));
            }
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            if stats {
                let (private, shared, resident) = tsrs_core::procs::writetrace::vm_breakdown();
                eprintln!("procs\tvm child {slice}\tprivate {} MiB\tshared {} MiB\tresident {} MiB", private >> 20, shared >> 20, resident >> 20);
            }
            pause_point(&format!("child-{slice}"), true);
            let encode_file = |f: P<SourceFile>| *file_index.get(&f).expect("a diagnostic in a file of the program");
            let mut out = Vec::new();
            // Every file of the slice, warm-up files included: checking later files may have added to their
            // diagnostics (related information), as it would in a checker thread that owns the slice.
            let mine: Vec<usize> = (0..files.len()).filter(|&i| slices[i] == slice).collect();
            put_u32(&mut out, mine.len() as u32);
            for &i in &mine {
                put_u32(&mut out, i as u32);
                put_u32(&mut out, results[i].len() as u32);
                for &d in &results[i] {
                    tsrs_ast::encode_diagnostic(&mut out, d, &encode_file);
                }
            }
            put_u32(&mut out, globals.len() as u32);
            for &d in &globals {
                tsrs_ast::encode_diagnostic(&mut out, d, &encode_file);
            }
            let counts = counts_of(&guard);
            for v in [counts.0 - base_counts.0, counts.1 - base_counts.1, counts.2 - base_counts.2] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            put_u32(&mut out, checked_files);
            out.extend_from_slice(&wall.to_le_bytes());
            tsrs_core::procs::self_stats().encode(&mut out);
            out
        };
        let child_count = n - first_child;
        // SAFETY: this thread is the only one running: the rayon pools are between jobs (program construction and
        // the file assignment have finished), the thread that started the command line waits in `join`, and no other
        // checker exists in this mode. The children only check files with this thread's checker, which uses no
        // thread pool and takes no lock another thread could hold (checkerpool_procs.rs, "Fork safety" in the note).
        let children = unsafe { tsrs_core::procs::fork_children(child_count, &mut work) }.unwrap_or_else(|e| panic!("tsrs: {e}"));
        let fork_times = children.fork_times.clone();
        let fork_done = start.elapsed().as_secs_f64();

        // The parent checks slice 0 while a thread reads the children's pipes.
        let (outputs, parent_info) = std::thread::scope(|s| {
            let reader = s.spawn(move || children.join());
            let mut parent_info = (0u32, 0.0f64, 0.0f64);
            if parent_checks {
                let parent_start = std::time::Instant::now();
                let cpu = thread_cpu_seconds();
                for i in 0..files.len() {
                    if slices[i] == 0 && !in_warmup[i] {
                        results[i] = collect(&mut guard, files[i]);
                        parent_info.0 += 1;
                    }
                }
                parent_info.1 = parent_start.elapsed().as_secs_f64();
                parent_info.2 = thread_cpu_seconds() - cpu;
            }
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            if stats {
                let (private, shared, resident) = tsrs_core::procs::writetrace::vm_breakdown();
                eprintln!("procs\tvm parent\tprivate {} MiB\tshared {} MiB\tresident {} MiB", private >> 20, shared >> 20, resident >> 20);
            }
            pause_point("parent", false);
            (reader.join().expect("checker process reader"), parent_info)
        });
        let outputs = outputs.unwrap_or_else(|e| panic!("tsrs: internal error: {e}"));

        let mut shared = procs.results.lock().unwrap();
        let mut report = String::new();
        let (mut child_cpu, mut child_instr, mut child_peak, mut child_pss, mut child_dirty) = (0.0, 0u64, 0u64, 0u64, 0u64);
        for (k, output) in outputs.iter().enumerate() {
            let mut input: &[u8] = &output.data;
            let file_at = |i: u32| files[i as usize];
            for _ in 0..get_u32(&mut input) {
                let i = get_u32(&mut input) as usize;
                let count = get_u32(&mut input);
                results[i] = (0..count).map(|_| tsrs_ast::decode_diagnostic(&mut input, &file_at)).collect();
            }
            for _ in 0..get_u32(&mut input) {
                shared.globals.push(tsrs_ast::decode_diagnostic(&mut input, &file_at));
            }
            let (t, s, inst) = (get_u64(&mut input), get_u64(&mut input), get_u64(&mut input));
            shared.counts.0 += t;
            shared.counts.1 += s;
            shared.counts.2 += inst;
            let checked_files = get_u32(&mut input);
            let wall = f64::from_le_bytes(input[..8].try_into().unwrap());
            input = &input[8..];
            let self_stats = tsrs_core::procs::SelfStats::decode(&mut input);
            child_cpu += output.cpu_seconds;
            child_instr += self_stats.instructions;
            child_peak += self_stats.peak_phys_footprint;
            child_pss += self_stats.pss;
            child_dirty += self_stats.private_dirty;
            if stats {
                use std::fmt::Write;
                let _ = writeln!(
                    report,
                    "procs\tchild {}\tfiles {checked_files}\twall {wall:.3}\tcpu {:.3}\tminflt {}\tinstr {:.1}G\tpeak footprint {} MiB\tpss {} MiB\tprivate dirty {} MiB\tshared {} MiB\trss {} MiB\tfork {:.1} ms",
                    first_child + k,
                    output.cpu_seconds,
                    output.minor_faults,
                    self_stats.instructions as f64 / 1e9,
                    self_stats.peak_phys_footprint >> 20,
                    self_stats.pss >> 20,
                    self_stats.private_dirty >> 20,
                    self_stats.shared >> 20,
                    self_stats.rss >> 20,
                    fork_times[k].as_secs_f64() * 1e3,
                );
            }
        }
        // The parent's checker (the pool's only one) went on with slice 0, so its own counters include that work.
        drop(guard);
        if stats {
            let me = tsrs_core::procs::self_stats();
            eprintln!(
                "procs\tprocesses {n}\twarm-up files {} (checked {})\twarm-up wall {warm_wall:.3}\twarm-up cpu {warm_cpu:.3}\tforks done {fork_done:.3}\tparent slice files {} wall {:.3} cpu {:.3}\ttotal {:.3}\tchildren cpu {child_cpu:.3}\tchildren instr {:.1}G\tchildren peak footprint {} MiB\tchildren pss {} MiB\tchildren private dirty {} MiB\tparent footprint {} MiB peak {} MiB instr {:.1}G",
                warmup.len(),
                checked.iter().filter(|&&c| c).count(),
                parent_info.0,
                parent_info.1,
                parent_info.2,
                start.elapsed().as_secs_f64(),
                child_instr as f64 / 1e9,
                child_peak >> 20,
                child_pss >> 20,
                child_dirty >> 20,
                me.phys_footprint >> 20,
                me.peak_phys_footprint >> 20,
                me.instructions as f64 / 1e9,
            );
            eprint!("{report}");
        }
        results
    }

    // Global diagnostics the children found (each child sends its checker's whole list; the caller deduplicates).
    pub(crate) fn process_globals(&self) -> Vec<P<Diagnostic>> {
        match self.state().processes.as_ref() {
            Some(procs) => procs.results.lock().unwrap().globals.clone(),
            None => Vec::new(),
        }
    }

    // Types, symbols and instantiations the children created after the fork.
    pub(crate) fn process_counts(&self) -> (u64, u64, u64) {
        match self.state().processes.as_ref() {
            Some(procs) => procs.results.lock().unwrap().counts,
            None => (0, 0, 0),
        }
    }

    pub(crate) fn uses_processes(&self) -> bool {
        self.processes > 0
    }

    pub(crate) fn processes_pending(&self) -> bool {
        self.processes > 0 && !self.state().processes.as_ref().is_some_and(|p| p.forked.load(Ordering::Relaxed))
    }
}

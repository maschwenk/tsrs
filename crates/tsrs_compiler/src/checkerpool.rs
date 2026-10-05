use std::ptr::NonNull;
use std::sync::{Mutex, MutexGuard, OnceLock};

use rustc_hash::FxHashMap;
use tsrs_ast::{self as ast, Diagnostic, SourceFile};
use tsrs_core::P;

use crate::program::{sort_and_deduplicate_diagnostics, Program};

#[cfg(feature = "checker")]
pub use tsrs_checker::{Checker, Context};

#[cfg(feature = "checker")]
fn new_checker(program: &'static Program) -> Box<Checker> {
    tsrs_checker::new_checker(program)
}

// Built without the checker crate: programs can be created, parsed and bound, but nothing can
// be type checked.
#[cfg(not(feature = "checker"))]
pub struct Checker {
    pub type_count: u32,
    pub symbol_count: u32,
    pub total_instantiation_count: u32,
    pub lazy_member_stats: tsrs_core::lazymembers::LazyMemberStats,
}

#[cfg(not(feature = "checker"))]
pub use tsrs_core::context::Context;

#[cfg(not(feature = "checker"))]
impl Checker {
    pub fn get_diagnostics_exported(&mut self, _ctx: &Context, _source_file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        unimplemented!("tsrs_compiler was built without the `checker` feature")
    }

    pub fn get_suggestion_diagnostics(&mut self, _ctx: &Context, _source_file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        unimplemented!("tsrs_compiler was built without the `checker` feature")
    }

    pub fn get_global_diagnostics(&mut self) -> Vec<P<Diagnostic>> {
        unimplemented!("tsrs_compiler was built without the `checker` feature")
    }

    pub fn was_canceled(&mut self) -> bool {
        false
    }
}

#[cfg(not(feature = "checker"))]
fn new_checker(_program: &'static Program) -> Box<Checker> {
    Box::new(Checker { type_count: 0, symbol_count: 0, total_instantiation_count: 0, lazy_member_stats: Default::default() })
}

// checkerpool.go:24
// CheckerPool is implemented by the project system to provide checkers with
// request-scoped lifetime and reclamation. It returns a checker and a release
// function that must be called when the caller is done with the checker.
// The returned checker must not be accessed concurrently; each acquisition is exclusive.
// Acquisitions are not reentrant, even when they share a request ID. Callers must
// pass an already acquired checker to nested operations instead of acquiring again.
// If file is non-nil, the pool may use it as an affinity hint to return the same
// checker for the same file across calls.
//
// Go returns `(*checker.Checker, func())`; the Rust pool returns a `CheckerHandle` that derefs to the checker
// and runs the release function when dropped (or on `release()`).
pub trait CheckerPool: Send + Sync {
    fn get_checker(&self, ctx: &Context, file: Option<P<SourceFile>>) -> CheckerHandle;
}

// Go's `(*checker.Checker, func())` pair. Dropping the handle is Go's `done()`; like Go's `sync.OnceFunc`
// releases it runs exactly once.
pub struct CheckerHandle {
    kind: checkerHandleKind,
}

enum checkerHandleKind {
    // The built-in pool's checkers live behind a mutex per checker (Go `locks[i]`).
    Locked(MutexGuard<'static, Box<Checker>>),
    // A checker owned by an external pool, which tracks exclusivity itself (Go project pool's `heldBy`).
    External { checker: NonNull<Checker>, release: Option<Box<dyn FnOnce()>> },
}

impl CheckerHandle {
    fn locked(guard: MutexGuard<'static, Box<Checker>>) -> CheckerHandle {
        CheckerHandle { kind: checkerHandleKind::Locked(guard) }
    }

    /// Hands out a checker owned by a pool outside this crate; `release` runs when the handle is dropped.
    ///
    /// # Safety
    /// Until `release` runs, the caller guarantees that `checker` stays alive and that nothing else accesses it
    /// (the pool has marked it held, Go `heldBy[i] = requestID`).
    pub unsafe fn from_raw(checker: NonNull<Checker>, release: impl FnOnce() + 'static) -> CheckerHandle {
        CheckerHandle { kind: checkerHandleKind::External { checker, release: Some(Box::new(release)) } }
    }

    // Go `done()`.
    pub fn release(self) {}
}

impl std::ops::Deref for CheckerHandle {
    type Target = Checker;
    fn deref(&self) -> &Checker {
        match &self.kind {
            checkerHandleKind::Locked(guard) => guard,
            // SAFETY: `from_raw`'s contract: the checker is alive and exclusively ours until release.
            checkerHandleKind::External { checker, .. } => unsafe { checker.as_ref() },
        }
    }
}

impl std::ops::DerefMut for CheckerHandle {
    fn deref_mut(&mut self) -> &mut Checker {
        match &mut self.kind {
            checkerHandleKind::Locked(guard) => guard,
            // SAFETY: `from_raw`'s contract: the checker is alive and exclusively ours until release.
            checkerHandleKind::External { checker, .. } => unsafe { checker.as_mut() },
        }
    }
}

impl Drop for CheckerHandle {
    fn drop(&mut self) {
        if let checkerHandleKind::External { release, .. } = &mut self.kind {
            if let Some(release) = release.take() {
                release();
            }
        }
    }
}

// A checker owned by a pool outside this crate (Go's project pool keeps `[]*checker.Checker` behind its mutex).
// `Checker` is not `Send` (it keeps `Rc`s and non-`Send` deferred closures, all reachable only from the checker
// itself), so a pool that shares checkers between threads stores them in this wrapper and, like the built-in pool,
// lets only the thread that holds a checker touch it.
pub struct PooledChecker(Box<Checker>);
#[expect(clippy::non_send_fields_in_send_ty, reason = "the checker: only the thread that holds it touches it (see above)")]
// SAFETY: the checker's `Rc`s and closures are reachable only from the checker, and the pool lets one thread at a time
// hold it (see above), so moving it to that thread moves all of them together.
unsafe impl Send for PooledChecker {}
// SAFETY: a `&PooledChecker` is only used by the thread that holds the checker (see above).
unsafe impl Sync for PooledChecker {}

impl PooledChecker {
    pub fn new(checker: Box<Checker>) -> PooledChecker {
        PooledChecker(checker)
    }

    // The checker's stable address, for `CheckerHandle::from_raw`.
    pub fn as_non_null(&mut self) -> NonNull<Checker> {
        NonNull::from(&mut *self.0)
    }

    pub fn into_inner(self) -> Box<Checker> {
        self.0
    }
}

impl std::ops::Deref for PooledChecker {
    type Target = Checker;
    fn deref(&self) -> &Checker {
        &self.0
    }
}

impl std::ops::DerefMut for PooledChecker {
    fn deref_mut(&mut self) -> &mut Checker {
        &mut self.0
    }
}

// Checkers recurse deeply (the single-threaded CLI runs on a 512 MB stack); each checker thread gets the same.
pub const CHECKER_STACK_SIZE: usize = 512 << 20;

// Go core.WorkGroup as used by the checker pool: runs `task(i)` for every index, each on its own OS thread,
// and waits for all of them. Single-threaded runs execute the tasks in order on the calling thread.
fn run_work_group(single_threaded: bool, count: usize, task: impl Fn(usize) + Sync) {
    if single_threaded || count <= 1 {
        (0..count).for_each(task);
        return;
    }
    std::thread::scope(|s| {
        let task = &task;
        let handles: Vec<_> = (0..count)
            .map(|i| {
                std::thread::Builder::new()
                    .name(format!("checker-{i}"))
                    .stack_size(CHECKER_STACK_SIZE)
                    .spawn_scoped(s, move || task(i))
                    .expect("failed to spawn checker thread")
            })
            .collect();
        let mut panic = None;
        for handle in handles {
            if let Err(payload) = handle.join() {
                panic.get_or_insert(payload);
            }
        }
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
    });
}

// A checker is mutated only while its mutex is held, by exactly one thread at a time; the pool
// never hands out references that outlive the guard. The checker's deferred closures are not
// `Send`, which is the only reason this wrapper is needed.
struct CheckerSlot(Mutex<Box<Checker>>);
#[expect(clippy::non_send_fields_in_send_ty, reason = "the checker: touched only under its mutex (see above)")]
// SAFETY: the checker's non-`Send` parts are reachable only from the checker, which is reached only through the mutex.
unsafe impl Send for CheckerSlot {}
// SAFETY: every access to the checker holds its mutex, and no reference outlives the guard (see above).
unsafe impl Sync for CheckerSlot {}

// A pool is dropped only with its program (`free_program` / `free_unshared_program`: no checker handle is held
// any more), so the leaked checkers can be freed with it. Programs that are never freed (the CLI) never get here.
impl Drop for poolState {
    fn drop(&mut self) {
        // SAFETY: `checkers` came from `Box::leak` of a boxed slice in `create_checkers`, and no handle borrowing a
        // checker outlives the pool's program.
        unsafe { drop(Box::from_raw(std::ptr::from_ref::<[CheckerSlot]>(self.checkers).cast_mut())) };
    }
}

pub(crate) struct poolState {
    // Leaked like the program that owns the pool, so a handle can hold a checker's lock without borrowing the pool.
    checkers: &'static [CheckerSlot],
    file_associations: FxHashMap<P<SourceFile>, usize>,
    // Checker index per program file index.
    pub(crate) associations: Vec<usize>,
    // TSRS_ASSIGNMENT_STATS only: per for_each_checker_group_do call, (seconds, files run) per checker.
    pub(crate) group_runs: Mutex<Vec<Vec<(f64, usize)>>>,
    // TSRS_ASSIGNMENT_STATS only: per for_each_checker_group_do call, thread CPU seconds per checker.
    pub(crate) group_cpu: Mutex<Vec<Vec<f64>>>,
    // TSRS_FILE_TIMES only: (file, checker, seconds, thread CPU seconds) per checked file, in completion order.
    pub(crate) file_times: Mutex<Vec<(P<SourceFile>, usize, f64, f64)>>,
    // Cost cache only: (file, thread CPU seconds) per checked file, per checker pass.
    file_cpu: Mutex<Vec<(P<SourceFile>, f64)>>,
}

// TSRS_FILE_TIMES=<path> (experiments): after checking, write one line per file run by a checker group:
// checker, seconds, thread CPU seconds, node count, text length, import count, file name, and the file's node-kind
// histogram (`Kind=count` pairs), for fitting assignment cost models offline.
pub(crate) fn file_times_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| std::env::var("TSRS_FILE_TIMES").ok().filter(|v| !v.is_empty())).as_deref()
}

pub(crate) fn write_file_times(program: &'static Program) {
    use std::fmt::Write;
    let (Some(path), Some(state)) = (file_times_path(), program.compiler_checker_pool().and_then(|pool| pool.state.get())) else {
        return;
    };
    fn count_kinds(node: P<tsrs_ast::Node>, counts: &mut [u32]) {
        counts[node.kind() as usize] += 1;
        node.for_each_child(&mut |child| {
            count_kinds(child, counts);
            false
        });
    }
    let mut out = String::new();
    for &(file, checker, seconds, cpu) in state.file_times.lock().unwrap().iter() {
        let mut counts = vec![0u32; tsrs_ast::Kind::Count as usize + 1];
        count_kinds(file.as_node(), &mut counts);
        let _ = write!(out, "{checker}\t{seconds:.6}\t{cpu:.6}\t{}\t{}\t{}\t{}\t", file.node_count.get(), file.text().len(), file.imports().len(), file.file_name());
        for (kind, &count) in counts.iter().enumerate().filter(|(_, &c)| c > 0) {
            let _ = write!(out, "{:?}={count} ", tsrs_ast::Kind::from_i16(kind as i16));
        }
        out.push('\n');
    }
    std::fs::write(path, out).expect("TSRS_FILE_TIMES");
}

// CPU time of the calling thread (TSRS_ASSIGNMENT_STATS): unlike wall time it does not grow when other
// processes take the cores, so per-checker balance can be compared on a loaded machine.
#[cfg(unix)]
pub(crate) fn thread_cpu_seconds() -> f64 {
    #[repr(C)]
    struct Timespec {
        tv_sec: i64,
        tv_nsec: i64,
    }
    extern "C" {
        fn clock_gettime(clock_id: i32, tp: *mut Timespec) -> i32;
    }
    #[cfg(target_os = "macos")]
    const CLOCK_THREAD_CPUTIME_ID: i32 = 16;
    #[cfg(not(target_os = "macos"))]
    const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
    let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: clock_gettime writes one timespec through the valid pointer.
    if unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &raw mut ts) } != 0 {
        return 0.0;
    }
    ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9
}

#[cfg(not(unix))]
pub(crate) fn thread_cpu_seconds() -> f64 {
    0.0
}

// TSRS_ASSIGNMENT_STATS=1: record per-checker group timings and, after `--extendedDiagnostics`, print the
// per-checker assignment report (checkerpool_stats.rs). Read once; nothing is recorded when unset.
pub fn assignment_stats_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TSRS_ASSIGNMENT_STATS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

pub(crate) struct checkerPool {
    program: &'static Program,
    checker_count: usize,
    single_threaded: bool,
    state: OnceLock<poolState>,
}

/*
Checker association is a balanced graph-partitioning problem:

  - A vertex is a source file.
  - An undirected edge connects two files for each resolved, in-program import
    entry between them. Multiple entries may connect the same pair and therefore
    strengthen their affinity. Self-imports and unresolved or external targets do
    not create edges.
  - A partition is a checker with its own symbol, type, and instantiation caches.

Putting related files on the same checker reduces duplicated cache construction,
but concentrating too many roots on one checker increases the parallel critical
path. We use weighted FENNEL to trade off those objectives:

  affinity(partition) - alpha * incrementalLoadPenalty(partition)

See Tsourakakis et al., "FENNEL: Streaming Graph Partitioning for Massive Scale
Graphs", WSDM 2014. FENNEL is sensitive to stream order, so stream order is part
of the policy below, rather than an incidental implementation detail.
*/

// These are empirical, project-independent operating points (see the Go source for the sweep).
const CHECKER_ASSOCIATION_TEXT_WEIGHT_DIVISOR: i64 = 100;
const CHECKER_ASSOCIATION_SOURCE_FILE_WEIGHT_MULTIPLIER: i64 = 4;
const CHECKER_ASSOCIATION_BALANCE_PENALTY_MULTIPLIER: i64 = 16;
const CHECKER_ASSOCIATION_PRIORITIZED_SOURCE_PENALTY: i64 = 12;
const CHECKER_ASSOCIATION_STRONG_BALANCE_MIN_CHECKER_COUNT: usize = 4;

struct checkerAssociationPolicy {
    prioritize_source_files: bool,
    source_file_weight_multiplier: i64,
    balance_penalty_multiplier: i64,
}

fn get_checker_association_policy(total_weight: i64, declaration_weight: i64, checker_count: usize) -> checkerAssociationPolicy {
    if should_prioritize_source_files(total_weight, declaration_weight, checker_count) {
        return checkerAssociationPolicy {
            prioritize_source_files: true,
            source_file_weight_multiplier: 1,
            balance_penalty_multiplier: CHECKER_ASSOCIATION_PRIORITIZED_SOURCE_PENALTY,
        };
    }
    if checker_count >= CHECKER_ASSOCIATION_STRONG_BALANCE_MIN_CHECKER_COUNT {
        return checkerAssociationPolicy {
            prioritize_source_files: false,
            source_file_weight_multiplier: CHECKER_ASSOCIATION_SOURCE_FILE_WEIGHT_MULTIPLIER,
            balance_penalty_multiplier: CHECKER_ASSOCIATION_BALANCE_PENALTY_MULTIPLIER,
        };
    }
    checkerAssociationPolicy { prioritize_source_files: false, source_file_weight_multiplier: 1, balance_penalty_multiplier: 1 }
}

// getCheckerAssociationsInOrder partitions the import graph using a weighted adaptation
// of FENNEL's streaming graph-partitioning objective with gamma = 3/2. A None order means
// stable program order. Ties are deterministic.
fn get_checker_associations_in_order(
    file_weights: &[i64],
    adjacent_files: &[Vec<usize>],
    file_order: Option<&[usize]>,
    checker_count: usize,
    penalty_multiplier: i64,
) -> Vec<usize> {
    if file_weights.is_empty() {
        return Vec::new();
    }

    let mut total_weight: i64 = 0;
    let mut max_file_weight: i64 = 0;
    let mut edge_count: i64 = 0;
    for (i, &weight) in file_weights.iter().enumerate() {
        total_weight += weight;
        max_file_weight = max_file_weight.max(weight);
        edge_count += adjacent_files[i].len() as i64;
    }

    let mut associations: Vec<i64> = vec![-1; file_weights.len()];
    let mut checker_weights: Vec<i64> = vec![0; checker_count];
    let checker_count_i = checker_count as i64;
    let average_checker_weight = (total_weight + checker_count_i - 1) / checker_count_i;
    let max_checker_weight = max_file_weight.max(average_checker_weight + average_checker_weight / 100);
    let total_weight_float = total_weight as f64;
    let alpha = penalty_multiplier as f64 * (edge_count / 2) as f64 * (checker_count as f64).sqrt()
        / (total_weight_float * total_weight_float.sqrt());
    let mut neighbor_counts: Vec<i64> = vec![0; checker_count];

    for position in 0..file_weights.len() {
        let file_index = match file_order {
            Some(order) => order[position],
            None => position,
        };

        neighbor_counts.iter_mut().for_each(|c| *c = 0);
        for &adjacent_file in &adjacent_files[file_index] {
            let checker_index = associations[adjacent_file];
            if checker_index >= 0 {
                neighbor_counts[checker_index as usize] += 1;
            }
        }

        let mut best_checker: i64 = -1;
        let mut best_score = f64::NEG_INFINITY;
        for (checker_index, &checker_weight) in checker_weights.iter().enumerate() {
            if checker_weight + file_weights[file_index] > max_checker_weight {
                continue;
            }
            let old_weight = checker_weight as f64;
            let new_weight = (checker_weight + file_weights[file_index]) as f64;
            let new_penalty = new_weight * new_weight.sqrt();
            let old_penalty = old_weight * old_weight.sqrt();
            let penalty = alpha * (new_penalty - old_penalty);
            let score = neighbor_counts[checker_index] as f64 - penalty;
            if score > best_score
                || score == best_score && (best_checker < 0 || checker_weight < checker_weights[best_checker as usize])
            {
                best_checker = checker_index as i64;
                best_score = score;
            }
        }
        if best_checker < 0 {
            best_checker = 0;
            for (checker_index, &checker_weight) in checker_weights[1..].iter().enumerate() {
                if checker_weight < checker_weights[best_checker as usize] {
                    best_checker = checker_index as i64 + 1;
                }
            }
        }
        associations[file_index] = best_checker;
        checker_weights[best_checker as usize] += file_weights[file_index];
    }
    associations.into_iter().map(|a| a as usize).collect()
}

// getCheckerAssociationOrder places source files before declarations and orders each group by
// descending estimated work. Returning None preserves program order.
fn get_checker_association_order(file_weights: &[i64], is_declaration_file: &[bool], prioritize_source_files: bool) -> Option<Vec<usize>> {
    if !prioritize_source_files {
        return None;
    }
    let mut file_order: Vec<usize> = (0..file_weights.len()).collect();
    file_order.sort_by(|&left, &right| {
        if is_declaration_file[left] != is_declaration_file[right] {
            return if !is_declaration_file[left] { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
        if file_weights[left] != file_weights[right] {
            return file_weights[right].cmp(&file_weights[left]);
        }
        left.cmp(&right)
    });
    Some(file_order)
}

fn get_checker_association_base_weight(node_count: i64, text_length: i64) -> i64 {
    (node_count + text_length / CHECKER_ASSOCIATION_TEXT_WEIGHT_DIVISOR).max(1)
}

// shouldPrioritizeSourceFiles reports whether all declaration-file base work is at
// most half of one average checker load.
fn should_prioritize_source_files(total_weight: i64, declaration_weight: i64, checker_count: usize) -> bool {
    declaration_weight * checker_count as i64 * 2 <= total_weight
}

// getCheckerAssociationWeights combines local syntax work with syntactic import fanout.
fn get_checker_association_weights(base_weights: &[i64], import_counts: &[i64]) -> Vec<i64> {
    let mut total_base_weight: i64 = 0;
    let mut total_imports: i64 = 0;
    for (i, &base_weight) in base_weights.iter().enumerate() {
        total_base_weight += base_weight;
        total_imports += import_counts[i];
    }
    let mut import_weight: i64 = 0;
    if total_imports > 0 {
        import_weight = (total_base_weight / total_imports).max(1);
    }
    base_weights.iter().enumerate().map(|(i, &base_weight)| base_weight + import_counts[i] * import_weight).collect()
}

impl checkerPool {
    // checkerpool.go:305 newCheckerPool / checkerpool.go:309 newCheckerPoolWithTracing (tracing is not ported).
    pub(crate) fn new(program: &'static Program) -> checkerPool {
        // Go's default is a constant 4; tsrs picks it per machine and program (default_checker_count).
        let checker_count: i64 = if program.single_threaded() {
            1
        } else if let Some(c) = program.options().checkers {
            c
        } else {
            default_checker_count(program)
        };

        // Go `max(min(checkerCount, len(files), 256), 1)` on int: a negative or zero count is one checker.
        let checker_count = checker_count.min(program.files.len() as i64).min(256).max(1) as usize;

        checkerPool { program, checker_count, single_threaded: program.single_threaded() || checker_count == 1, state: OnceLock::new() }
    }

    // checkerpool.go:331
    // GetChecker implements CheckerPool. When file is non-nil, returns the checker
    // associated with that file; otherwise returns the first checker.
    pub(crate) fn get_checker_exclusive(&self, file: Option<P<SourceFile>>) -> CheckerHandle {
        if let Some(file) = file {
            return self.get_checker_for_file_exclusive(file);
        }
        let state = self.create_checkers();
        CheckerHandle::locked(state.checkers[0].0.lock().unwrap())
    }

    // checkerpool.go:346
    // getCheckerForFileNonExclusive returns the checker for the given file without locking.
    // This is only safe when the caller guarantees no concurrent access to the same checker,
    // e.g. for read-only operations like obtaining an emit resolver.
    // Rust hands out `&mut Checker` only under the checker's lock, so this locks like the exclusive variant.
    pub(crate) fn get_checker_for_file_non_exclusive(&self, file: P<SourceFile>) -> CheckerHandle {
        self.get_checker_for_file_exclusive(file)
    }

    // checkerpool.go:351
    pub(crate) fn get_checker_for_file_exclusive(&self, file: P<SourceFile>) -> CheckerHandle {
        let state = self.create_checkers();
        let idx = state.file_associations[&file];
        CheckerHandle::locked(state.checkers[idx].0.lock().unwrap())
    }

    // checkerpool.go:362
    // getCheckerNonExclusive returns the first checker without locking (locks in Rust, see above).
    pub(crate) fn get_checker_non_exclusive(&self) -> CheckerHandle {
        let state = self.create_checkers();
        CheckerHandle::locked(state.checkers[0].0.lock().unwrap())
    }

    // checkerpool.go:367
    fn create_checkers(&self) -> &poolState {
        let program = self.program;
        self.state.get_or_init(|| {
            if tsrs_core::ptr::shared_check::enabled() {
                // Debug aid: bind up front so every parser/binder allocation is recorded as shared.
                tsrs_core::ptr::shared_check::thaw();
                program.bind_source_files();
                tsrs_core::ptr::shared_check::freeze_shared_objects();
            }
            let create_start = std::time::Instant::now();
            #[cfg(feature = "checker")]
            tsrs_checker::links::set_multiple_checkers(self.checker_count > 1);
            let slots: Vec<Mutex<Option<CheckerSlot>>> = (0..self.checker_count).map(|_| Mutex::new(None)).collect();
            run_work_group(self.single_threaded, self.checker_count, |i| {
                *slots[i].lock().unwrap() = Some(CheckerSlot(Mutex::new(new_checker(program))));
            });
            let checkers: &'static [CheckerSlot] =
                Box::leak(slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect::<Vec<_>>().into_boxed_slice());
            tsrs_core::phases::record("Checkers: create", create_start.elapsed());

            let files = &program.files;
            let associations = tsrs_core::phases::time("Checkers: assign files", || compute_associations(program, self.checker_count));
            let mut file_associations = FxHashMap::default();
            for (i, &file) in files.iter().enumerate() {
                file_associations.insert(file, associations[i]);
            }
            poolState {
                checkers,
                file_associations,
                associations,
                group_runs: Mutex::new(Vec::new()),
                group_cpu: Mutex::new(Vec::new()),
                file_times: Mutex::new(Vec::new()),
                file_cpu: Mutex::new(Vec::new()),
            }
        })
    }

    pub(crate) fn state(&self) -> &poolState {
        self.create_checkers()
    }

    pub(crate) fn checker_count(&self) -> usize {
        self.checker_count
    }

    pub(crate) fn checker_index_of_file(&self, file: P<SourceFile>) -> Option<usize> {
        self.create_checkers().file_associations.get(&file).copied()
    }

    // checkerpool.go:451
    // Runs `cb` for each checker in the pool concurrently, locking and unlocking checker mutexes as it goes,
    // making it safe to call `forEachCheckerParallel` from many threads simultaneously.
    pub(crate) fn for_each_checker_parallel(&self, cb: impl Fn(usize, &mut Checker) + Sync) {
        let state = self.create_checkers();
        let run = |idx: usize| {
            let mut guard = state.checkers[idx].0.lock().unwrap();
            cb(idx, &mut guard);
        };
        run_work_group(self.single_threaded, state.checkers.len(), run);
    }

    // checkerpool.go:464
    pub(crate) fn get_global_diagnostics(&self) -> Vec<P<Diagnostic>> {
        let state = self.create_checkers();
        let global_diagnostics: Vec<Mutex<Vec<P<Diagnostic>>>> = (0..state.checkers.len()).map(|_| Mutex::new(Vec::new())).collect();
        self.for_each_checker_parallel(|idx, checker| {
            *global_diagnostics[idx].lock().unwrap() = checker.get_global_diagnostics();
        });
        let all: Vec<P<Diagnostic>> = global_diagnostics.into_iter().flat_map(|d| d.into_inner().unwrap()).collect();
        sort_and_deduplicate_diagnostics(&all)
    }

    // checkerpool.go:476
    // forEachCheckerGroupDo runs one task per checker in parallel. Each task iterates the provided files,
    // processing only those assigned to its checker. Within each checker's set, files are
    // visited in their original order (load-bearing: another order can change the property order of
    // types printed in messages, notes/perf-balance.md).
    pub(crate) fn for_each_checker_group_do(
        &self,
        files: &[P<SourceFile>],
        single_threaded: bool,
        cb: impl Fn(&mut Checker, usize, P<SourceFile>) + Sync,
    ) {
        let state = self.create_checkers();
        let stats = assignment_stats_enabled();
        let times: Vec<Mutex<(f64, usize)>> = if stats { (0..state.checkers.len()).map(|_| Mutex::new((0.0, 0))).collect() } else { Vec::new() };
        let cpu: Vec<Mutex<f64>> = if stats { (0..state.checkers.len()).map(|_| Mutex::new(0.0)).collect() } else { Vec::new() };
        let file_times = file_times_path().is_some();
        let cost_cache = checker_cost_cache_path().is_some() && state.checkers.len() > 1;
        let run = |checker_idx: usize| {
            let start = stats.then(std::time::Instant::now);
            let cpu_start = if stats { thread_cpu_seconds() } else { 0.0 };
            let mut count = 0;
            let mut file_cpu: Vec<(P<SourceFile>, f64)> = Vec::new();
            let mut guard = state.checkers[checker_idx].0.lock().unwrap();
            for i in visit_order(files.len()) {
                let file = files[i];
                if state.file_associations.get(&file) == Some(&checker_idx) {
                    let file_start = file_times.then(std::time::Instant::now);
                    let cpu_start = if cost_cache || file_times { thread_cpu_seconds() } else { 0.0 };
                    cb(&mut guard, i, file);
                    if let Some(file_start) = file_start {
                        let cpu = thread_cpu_seconds() - cpu_start;
                        state.file_times.lock().unwrap().push((file, checker_idx, file_start.elapsed().as_secs_f64(), cpu));
                    }
                    if cost_cache {
                        file_cpu.push((file, thread_cpu_seconds() - cpu_start));
                    }
                    count += 1;
                }
            }
            if cost_cache {
                state.file_cpu.lock().unwrap().extend(file_cpu);
            }
            if let Some(start) = start {
                *times[checker_idx].lock().unwrap() = (start.elapsed().as_secs_f64(), count);
                *cpu[checker_idx].lock().unwrap() = thread_cpu_seconds() - cpu_start;
            }
        };
        // Go queues one goroutine per checker group (cheap); here each group is an OS thread, so spawn threads only for
        // the checkers that own at least one of `files`. A one-file call (incremental emit of one affected file) then
        // runs on the calling thread instead of creating `checkers.len()` threads per file.
        let active: Vec<usize> = (0..state.checkers.len()).filter(|&i| files.iter().any(|f| state.file_associations.get(f) == Some(&i))).collect();
        run_work_group(single_threaded || self.single_threaded, active.len(), |k| run(active[k]));
        if stats {
            state.group_runs.lock().unwrap().push(times.into_iter().map(|t| t.into_inner().unwrap()).collect());
            state.group_cpu.lock().unwrap().push(cpu.into_iter().map(|t| t.into_inner().unwrap()).collect());
        }
    }
}

impl CheckerPool for checkerPool {
    fn get_checker(&self, _ctx: &Context, file: Option<P<SourceFile>>) -> CheckerHandle {
        self.get_checker_exclusive(file)
    }
}

// The order in which a checker group visits `count` files: program order, or with `TSRS_CHECKER_ASSIGNMENT=random:<seed>`
// a permutation drawn from the seed (a debug mode that checks that output does not depend on the visit order).
fn visit_order(count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    if let CheckerAssignment::Random(seed) = checker_assignment() {
        // Fisher-Yates with splitmix64.
        let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
        for i in (1..count).rev() {
            let j = (splitmix64(&mut state) % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
    }
    order
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

// Go `createCheckers`' association step (FENNEL over the import graph, see above).
fn go_associations(program: &Program, checker_count: usize) -> Vec<usize> {
    let files = &program.files;
    let mut base_weights = vec![0i64; files.len()];
    let mut import_counts = vec![0i64; files.len()];
    let mut is_declaration_file = vec![false; files.len()];
    let mut total_base_weight = 0i64;
    let mut declaration_base_weight = 0i64;
    for (i, file) in files.iter().enumerate() {
        let base_weight = get_checker_association_base_weight(file.node_count.get() as i64, file.text().len() as i64);
        total_base_weight += base_weight;
        if file.is_declaration_file.get() {
            declaration_base_weight += base_weight;
        }
        base_weights[i] = base_weight;
        import_counts[i] = file.imports().len() as i64;
        is_declaration_file[i] = file.is_declaration_file.get();
    }
    let policy = get_checker_association_policy(total_base_weight, declaration_base_weight, checker_count);
    if policy.source_file_weight_multiplier != 1 {
        // Apply this before import normalization. The policy intentionally
        // increases both source-file work and the normalized import unit.
        for (i, &declaration) in is_declaration_file.iter().enumerate() {
            if !declaration {
                base_weights[i] *= policy.source_file_weight_multiplier;
            }
        }
    }
    let file_weights = get_checker_association_weights(&base_weights, &import_counts);
    let adjacent_files = get_import_adjacency(program);
    let file_order = get_checker_association_order(&file_weights, &is_declaration_file, policy.prioritize_source_files);
    get_checker_associations_in_order(
        &file_weights,
        &adjacent_files,
        file_order.as_deref(),
        checker_count,
        policy.balance_penalty_multiplier,
    )
}

fn compute_associations(program: &Program, checker_count: usize) -> Vec<usize> {
    if checker_count <= 1 {
        return vec![0; program.files.len()];
    }
    let associations = match checker_assignment() {
        CheckerAssignment::Go => go_associations(program, checker_count),
        CheckerAssignment::Locality => locality_associations(program, checker_count),
        CheckerAssignment::Random(seed) => {
            let mut state = seed;
            (0..program.files.len()).map(|_| (splitmix64(&mut state) % checker_count as u64) as usize).collect()
        }
        CheckerAssignment::File(path) => {
            let text = std::fs::read_to_string(path).expect("TSRS_CHECKER_ASSIGNMENT file");
            let associations: Vec<usize> = text.lines().map(|l| l.trim().parse::<usize>().unwrap().min(checker_count - 1)).collect();
            assert_eq!(associations.len(), program.files.len(), "TSRS_CHECKER_ASSIGNMENT file length");
            associations
        }
    };
    if let Ok(path) = std::env::var("TSRS_ASSIGNMENT_DUMP") {
        dump_assignment_inputs(program, &associations, &path);
    }
    associations
}

// tsrs-only, opt-in: `--checkerCostCache <file>` (CLI) or TSRS_CHECKER_COST_CACHE=<file>. The locality
// assignment balances the checkers on per-file check seconds measured by a previous run (read from the file
// when it exists) instead of the syntactic weight, and the run writes its own measurements back. Without
// the option nothing is read or written and the assignment depends only on the program.
static CLI_CHECKER_COST_CACHE: OnceLock<String> = OnceLock::new();

pub fn set_checker_cost_cache_from_cli(path: &str) {
    let _ = CLI_CHECKER_COST_CACHE.set(path.to_string());
}

// The cost cache path; None unless the option is set and the locality assignment is used.
fn checker_cost_cache_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        CLI_CHECKER_COST_CACHE
            .get()
            .cloned()
            .or_else(|| std::env::var("TSRS_CHECKER_COST_CACHE").ok())
            .filter(|p| !p.is_empty() && matches!(checker_assignment(), CheckerAssignment::Locality))
    })
    .as_deref()
}

const COST_CACHE_MAGIC: &str = "# tsrs checker cost cache v2";

// One cost cache entry: the check CPU seconds a file took in the previous run and the checker that ran it.
#[derive(Clone, Copy)]
struct CostEntry {
    seconds: f64,
    checker: usize,
}

// The cost cache: `<magic> checkers=<n>`, then `<seconds>\t<checker>\t<file path>` per checked file. Returns
// the entries and the previous run's checker count; a missing, unreadable or foreign file reads as empty, and
// malformed lines are skipped.
fn read_cost_cache(path: &str) -> (FxHashMap<String, CostEntry>, usize) {
    let mut entries = FxHashMap::default();
    let Ok(text) = std::fs::read_to_string(path) else {
        return (entries, 0);
    };
    let mut lines = text.lines();
    let Some(checker_count) = lines
        .next()
        .and_then(|header| header.strip_prefix(COST_CACHE_MAGIC))
        .and_then(|rest| rest.trim().strip_prefix("checkers="))
        .and_then(|n| n.parse::<usize>().ok())
    else {
        return (entries, 0);
    };
    for line in lines {
        let mut fields = line.splitn(3, '\t');
        let (Some(seconds), Some(checker), Some(file)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        let (Ok(seconds), Ok(checker)) = (seconds.parse::<f64>(), checker.parse::<usize>()) else {
            continue;
        };
        if seconds.is_finite() && seconds >= 0.0 && checker < checker_count {
            entries.insert(file.to_string(), CostEntry { seconds, checker });
        }
    }
    (entries, checker_count)
}

// Writes the cost cache when it is enabled and more than one checker ran: per type-checked file, its thread CPU
// seconds summed over the checker passes and its checker. Files of other runs are dropped.
pub(crate) fn write_cost_cache(program: &'static Program) {
    use std::fmt::Write;
    // Programs with an external checker pool (language server / API projects) have no cost state to write.
    let (Some(path), Some(state)) = (checker_cost_cache_path(), program.compiler_checker_pool().and_then(|pool| pool.state.get())) else {
        return;
    };
    if state.checkers.len() <= 1 {
        return;
    }
    let file_indices: FxHashMap<P<SourceFile>, usize> = program.files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    let mut measured: Vec<f64> = vec![-1.0; program.files.len()];
    for &(file, seconds) in state.file_cpu.lock().unwrap().iter() {
        if let Some(&i) = file_indices.get(&file) {
            measured[i] = measured[i].max(0.0) + seconds;
        }
    }
    let mut entries: Vec<(&str, f64, usize)> = Vec::new();
    for (i, file) in program.files.iter().enumerate() {
        let skipped = (file.is_declaration_file.get() || ast::is_json_source_file(*file)) && program.skip_type_checking(*file, false);
        if measured[i] >= 0.0 && !skipped {
            entries.push((file.path(), measured[i], state.associations[i]));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::with_capacity(entries.len() * 100);
    let _ = writeln!(out, "{COST_CACHE_MAGIC} checkers={}", state.checkers.len());
    for (file, seconds, checker) in entries {
        let _ = writeln!(out, "{seconds:.6}\t{checker}\t{file}");
    }
    // Write-then-rename so a concurrent reader never sees a partial file; on failure the old cache stays.
    let tmp = format!("{path}.tmp{}", std::process::id());
    if std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

// tsrs-only: how files are assigned to checkers. `--checkerAssignment <name>` (CLI) or
// TSRS_CHECKER_ASSIGNMENT=<name> (any binary). The assignment never changes what a file's diagnostics are, only
// which checker computes them (and so how much checker state is duplicated across checkers).
//   locality (default): directory-subtree groups packed onto checkers with Go's FENNEL (below)
//   go:                 Go's createCheckers association (FENNEL over single files in program order)
//   file:<path>:        one checker index per line by program file index (experiments)
//   random:<seed>:      a random checker per file and a random visit order in each checker, drawn from the seed
//                       (debug: output must not depend on the assignment)
pub enum CheckerAssignment {
    Locality,
    Go,
    File(String),
    Random(u64),
}

static CLI_CHECKER_ASSIGNMENT: OnceLock<String> = OnceLock::new();

/// `--checkerAssignment <name>`; returns false for an unknown name.
pub fn set_checker_assignment_from_cli(name: &str) -> bool {
    if parse_checker_assignment(name).is_none() {
        return false;
    }
    let _ = CLI_CHECKER_ASSIGNMENT.set(name.to_string());
    true
}

fn parse_checker_assignment(name: &str) -> Option<CheckerAssignment> {
    match name {
        "" | "locality" => Some(CheckerAssignment::Locality),
        "go" => Some(CheckerAssignment::Go),
        _ => match name.strip_prefix("random:") {
            Some(seed) => seed.parse::<u64>().ok().map(CheckerAssignment::Random),
            None => name.strip_prefix("file:").map(|p| CheckerAssignment::File(p.to_string())),
        },
    }
}

fn checker_assignment() -> CheckerAssignment {
    let name = match CLI_CHECKER_ASSIGNMENT.get() {
        Some(name) => name.clone(),
        None => std::env::var("TSRS_CHECKER_ASSIGNMENT").unwrap_or_default(),
    };
    parse_checker_assignment(&name).unwrap_or_else(|| panic!("unknown checker assignment {name:?} (locality, go, file:<path>, random:<seed>)"))
}

// A directory subtree whose checked weight is at most 1/LOCALITY_GROUP_FRACTION of an average checker load is
// kept on one checker.
const LOCALITY_GROUP_FRACTION: i64 = 4;
const LOCALITY_PENALTY_MULTIPLIER: i64 = 1;

// Go's checker count without --checkers.
const GO_DEFAULT_CHECKERS: i64 = 4;
// Past 8 checkers the duplicated first-touch work, the imbalance and the slower cores eat the gain while every checker
// adds memory (notes/perf-checker-scaling.md).
const MAX_DEFAULT_CHECKERS: i64 = 8;
// A checker beyond Go's 4 needs at least this many type-checked files to be worth its creation and duplicated work.
const MIN_CHECKED_FILES_PER_DEFAULT_CHECKER: i64 = 32;

static GO_DEFAULT_CHECKER_COUNT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Build mode runs up to 4 projects at once (Go's `--builders` default), each with its own pool, so it keeps Go's
/// constant default instead of sizing every pool for the whole machine.
pub fn use_go_default_checker_count() {
    GO_DEFAULT_CHECKER_COUNT.store(true, std::sync::atomic::Ordering::Relaxed);
}

// tsrs-only: the checker count when neither --checkers nor --singleThreaded is given. Go always uses 4. Here: half the
// available parallelism, at least Go's 4 and at most MAX_DEFAULT_CHECKERS, and no more than one checker per
// MIN_CHECKED_FILES_PER_DEFAULT_CHECKER type-checked files, so small programs keep Go's 4. Diagnostics do not depend
// on the count; the --extendedDiagnostics Types / Symbols / Instantiations counters do (each checker counts what it
// creates), so they now depend on the machine unless --checkers is given.
fn default_checker_count(program: &Program) -> i64 {
    if GO_DEFAULT_CHECKER_COUNT.load(std::sync::atomic::Ordering::Relaxed) {
        return GO_DEFAULT_CHECKERS;
    }
    let parallelism = std::thread::available_parallelism().map_or(1, |n| n.get()) as i64;
    let by_machine = (parallelism / 2).min(MAX_DEFAULT_CHECKERS);
    if by_machine <= GO_DEFAULT_CHECKERS {
        return GO_DEFAULT_CHECKERS;
    }
    // The files a checker does work for (the same rule as checked_file_weights).
    let checked =
        program.files.iter().filter(|&&f| !((f.is_declaration_file.get() || ast::is_json_source_file(f)) && program.skip_type_checking(f, false))).count();
    by_machine.min(checked as i64 / MIN_CHECKED_FILES_PER_DEFAULT_CHECKER).max(GO_DEFAULT_CHECKERS)
}

// Checker assignment by locality. Checker state duplication comes from files on different checkers that
// resolve the same declarations; files of one directory subtree (a feature folder, a package) resolve
// largely the same ones, more so than direct import edges predict. So: (1) only files that are type checked
// carry weight (Go's weights; skipLibCheck declaration files are never checked and would otherwise take a
// share of a checker's budget; see checked_file_weights), (2) every directory subtree of at most 1/LOCALITY_GROUP_FRACTION of an
// average checker load becomes one group (the shallowest such subtree per file; a file directly inside an
// oversized directory is its own group), (3) the groups are placed with Go's FENNEL step (affinity = import
// edges between groups, 101% load cap) in descending weight order, largest first. Deterministic: paths are
// sorted, groups are numbered in path order, ties are broken by index. Unchecked declaration files go to
// checker 0 (no checker ever runs over them).
fn locality_associations(program: &Program, checker_count: usize) -> Vec<usize> {
    let files = &program.files;
    let weights = checked_file_weights(program);
    let mut order: Vec<usize> = (0..files.len()).filter(|&i| weights[i] > 0).collect();
    let mut associations = vec![0usize; files.len()];
    if order.is_empty() {
        return associations;
    }
    order.sort_by(|&a, &b| files[a].path().cmp(files[b].path()).then(a.cmp(&b)));

    // With a cost cache, groups are formed and placed by measured cost (see measured_file_costs).
    let measured = checker_cost_cache_path().and_then(|path| measured_file_costs(&read_cost_cache(path), checker_count, files, &order, &weights));
    let costs: &[i64] = measured.as_ref().map_or(&weights, |m| &m.costs);

    let total: i64 = order.iter().map(|&i| costs[i]).sum();
    let threshold = total / (checker_count as i64 * LOCALITY_GROUP_FRACTION);

    // Checked weight of every directory subtree (keys are path prefixes ending before a '/').
    let mut subtree_weights: FxHashMap<&str, i64> = FxHashMap::default();
    for &i in &order {
        let path: &str = files[i].path();
        for (pos, _) in path.match_indices('/') {
            *subtree_weights.entry(&path[..pos]).or_default() += costs[i];
        }
    }
    let mut group_ids: FxHashMap<&str, usize> = FxHashMap::default();
    let mut group_of_file = vec![usize::MAX; files.len()];
    let mut group_weights: Vec<i64> = Vec::new();
    for &i in &order {
        let path: &str = files[i].path();
        let key = path.match_indices('/').map(|(pos, _)| &path[..pos]).find(|dir| subtree_weights[dir] <= threshold).unwrap_or(path);
        let group = *group_ids.entry(key).or_insert_with(|| {
            group_weights.push(0);
            group_weights.len() - 1
        });
        group_of_file[i] = group;
        group_weights[group] += costs[i];
    }

    // Import edges between groups of checked files, one adjacency entry per file-level edge and direction.
    let adjacent_files = get_import_adjacency(program);
    let mut group_adjacency: Vec<Vec<usize>> = vec![Vec::new(); group_weights.len()];
    for &i in &order {
        for &j in &adjacent_files[i] {
            let (gi, gj) = (group_of_file[i], group_of_file[j]);
            if gj != usize::MAX && gi != gj {
                group_adjacency[gi].push(gj);
            }
        }
    }

    let fennel = |group_weights: &[i64]| {
        let mut group_order: Vec<usize> = (0..group_weights.len()).collect();
        group_order.sort_by(|&a, &b| group_weights[b].cmp(&group_weights[a]).then(a.cmp(&b)));
        get_checker_associations_in_order(group_weights, &group_adjacency, Some(&group_order), checker_count, LOCALITY_PENALTY_MULTIPLIER)
    };
    let group_associations = match &measured {
        None => fennel(&group_weights),
        Some(measured) => {
            // Each group starts on the checker that ran most of its cached cost in the previous run, so that
            // costs are used in the context they were measured in; then a few groups are moved.
            let mut previous = vec![(usize::MAX, 0i64); group_weights.len()];
            if let Some(file_checkers) = &measured.checkers {
                let mut shares = vec![0i64; group_weights.len() * checker_count];
                for &i in &order {
                    if let Some(c) = file_checkers[i] {
                        shares[group_of_file[i] * checker_count + c] += costs[i].max(1);
                    }
                }
                for (g, previous) in previous.iter_mut().enumerate() {
                    for c in 0..checker_count {
                        if shares[g * checker_count + c] > previous.1 {
                            *previous = (c, shares[g * checker_count + c]);
                        }
                    }
                }
            }
            let mut result = if previous.iter().all(|p| p.0 == usize::MAX) { fennel(&group_weights) } else { vec![usize::MAX; group_weights.len()] };
            let mut loads = vec![0i64; checker_count];
            for g in 0..result.len() {
                if previous[g].0 != usize::MAX {
                    result[g] = previous[g].0;
                }
                if result[g] != usize::MAX {
                    loads[result[g]] += group_weights[g];
                }
            }
            // Groups without a previous checker (new files) go to the least loaded checker, largest first.
            let mut unplaced: Vec<usize> = (0..result.len()).filter(|&g| result[g] == usize::MAX).collect();
            unplaced.sort_by(|&a, &b| group_weights[b].cmp(&group_weights[a]).then(a.cmp(&b)));
            for g in unplaced {
                let c = (0..checker_count).min_by_key(|&c| (loads[c], c)).unwrap();
                result[g] = c;
                loads[c] += group_weights[g];
            }
            refine_group_associations(&mut result, &group_weights, &group_adjacency, checker_count);
            result
        }
    };
    for &i in &order {
        associations[i] = group_associations[group_of_file[i]];
    }
    associations
}

struct MeasuredCosts {
    // Per program file: the cached check cost in nanoseconds, or for files without an entry their static
    // weight converted at the average nanoseconds per weight unit of the files that have one.
    costs: Vec<i64>,
    // Per program file, the checker that ran it in the previous run (None without an entry); None when that
    // run used a different checker count.
    checkers: Option<Vec<Option<usize>>>,
}

// None when the cache has no entry for a checked file of this program (first run, other project, ...).
fn measured_file_costs(
    cache: &(FxHashMap<String, CostEntry>, usize),
    checker_count: usize,
    files: &[P<SourceFile>],
    order: &[usize],
    weights: &[i64],
) -> Option<MeasuredCosts> {
    let (entries, cached_checker_count) = cache;
    let mut cached: Vec<Option<CostEntry>> = vec![None; files.len()];
    let (mut cached_seconds, mut cached_weight) = (0.0f64, 0i64);
    for &i in order {
        cached[i] = entries.get(files[i].path() as &str).copied();
        if let Some(entry) = cached[i] {
            cached_seconds += entry.seconds;
            cached_weight += weights[i];
        }
    }
    if cached_weight == 0 || cached_seconds <= 0.0 {
        return None;
    }
    let ns_per_weight = cached_seconds * 1e9 / cached_weight as f64;
    let costs = (0..files.len())
        .map(|i| match cached[i] {
            _ if weights[i] == 0 => 0,
            Some(entry) => ((entry.seconds * 1e9) as i64).max(1),
            None => ((weights[i] as f64 * ns_per_weight) as i64).max(1),
        })
        .collect();
    let checkers = (*cached_checker_count == checker_count).then(|| cached.iter().map(|e| e.map(|e| e.checker)).collect());
    Some(MeasuredCosts { costs, checkers })
}

// A group moved to another checker shares less with its new neighbors than with its old ones: measured on the
// private monorepo, the destination checker gains ~1.3x the group's measured cost while the source loses ~1x.
const MOVE_COST_PREMIUM: f64 = 1.3;
const MAX_REFINE_MOVES: usize = 64;
const REFINE_START_IMBALANCE: f64 = 0.03;
const REFINE_STOP_IMBALANCE: f64 = 0.01;

// Moves single groups off the most loaded checker (by measured cost) while that lowers the predicted maximum
// load by more than 0.25% of the mean. Starting from the previous run's placement, few moves keep most groups in
// the context their costs were measured in, and later runs correct what a move mispredicted. Deterministic for a
// given cache: ties go to the stronger import affinity to the destination, then to the lower group and checker.
fn refine_group_associations(associations: &mut [usize], costs: &[i64], adjacency: &[Vec<usize>], checker_count: usize) {
    let mut loads = vec![0f64; checker_count];
    for (g, &c) in associations.iter().enumerate() {
        loads[c] += costs[g] as f64;
    }
    let mean = loads.iter().sum::<f64>() / checker_count as f64;
    let min_gain = mean * 0.0025;
    let mut affinity = vec![0i64; checker_count];
    for moves in 0..MAX_REFINE_MOVES {
        let heavy = (0..checker_count).max_by(|&a, &b| loads[a].total_cmp(&loads[b]).then(b.cmp(&a))).unwrap();
        // Measured loads vary by a few percent run to run: leave a placement alone unless its slowest checker is
        // REFINE_START_IMBALANCE above the mean, then move groups until it is within REFINE_STOP_IMBALANCE.
        let imbalance = loads[heavy] / mean - 1.0;
        if imbalance < if moves == 0 { REFINE_START_IMBALANCE } else { REFINE_STOP_IMBALANCE } {
            break;
        }
        let mut best: Option<(f64, i64, usize, usize)> = None; // (new max, -affinity gain, group, destination)
        for g in 0..costs.len() {
            if associations[g] != heavy {
                continue;
            }
            affinity.iter_mut().for_each(|a| *a = 0);
            for &n in &adjacency[g] {
                affinity[associations[n]] += 1;
            }
            let cost = costs[g] as f64;
            for dest in 0..checker_count {
                if dest == heavy {
                    continue;
                }
                let mut new_max = (loads[heavy] - cost).max(loads[dest] + cost * MOVE_COST_PREMIUM);
                for c in 0..checker_count {
                    if c != heavy && c != dest {
                        new_max = new_max.max(loads[c]);
                    }
                }
                let key = (new_max, affinity[heavy] - affinity[dest], g, dest);
                if best.is_none_or(|b| key.0 < b.0 || key.0 == b.0 && (key.1, key.2, key.3) < (b.1, b.2, b.3)) {
                    best = Some(key);
                }
            }
        }
        let Some((new_max, _, g, dest)) = best else { break };
        if new_max > loads[heavy] - min_gain {
            break;
        }
        loads[heavy] -= costs[g] as f64;
        loads[dest] += costs[g] as f64 * MOVE_COST_PREMIUM;
        associations[g] = dest;
    }
}

// Go's file weights (base work, regime-2 source multiplier, normalized import fanout) for the files a checker
// processes; 0 for declaration and JSON files that are not type checked (skipLibCheck / skipDefaultLibCheck;
// JSON files are never checked): no checker does work for them. Other source files keep their weight even
// when not type checked (noCheck, JS without checkJs), since declaration diagnostics still use their checker.
// Go's 4x source multiplier separates checked sources from declaration files that are mostly not checked; here
// unchecked files already weigh 0, so declaration files that are checked (no skipLibCheck) get it too. Measured
// on webpack (642 checked declaration files): 393 ns of check CPU per base unit for declaration files, 536 for
// sources; with 4 checkers the slowest checker (the one holding the lib files) went from 53% to 12% above the mean.
fn checked_file_weights(program: &Program) -> Vec<i64> {
    let files = &program.files;
    let checked: Vec<bool> = files
        .iter()
        .map(|&f| !((f.is_declaration_file.get() || ast::is_json_source_file(f)) && program.skip_type_checking(f, false)))
        .collect();
    let base_weights: Vec<i64> = files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            if !checked[i] {
                return 0;
            }
            let base = get_checker_association_base_weight(f.node_count.get() as i64, f.text().len() as i64);
            base * CHECKER_ASSOCIATION_SOURCE_FILE_WEIGHT_MULTIPLIER
        })
        .collect();
    let import_counts: Vec<i64> = files.iter().enumerate().map(|(i, f)| if checked[i] { f.imports().len() as i64 } else { 0 }).collect();
    get_checker_association_weights(&base_weights, &import_counts)
}

// TSRS_ASSIGNMENT_DUMP=<path>: writes the assignment inputs for offline experiments: `<path>.files.tsv`
// (index, checked, declaration, node count, text length, import count, association, file name) and
// `<path>.edges.tsv` (directed resolved in-program imports, by file index).
fn dump_assignment_inputs(program: &Program, associations: &[usize], path: &str) {
    use std::fmt::Write;
    let files = &program.files;
    let mut out = String::new();
    for (i, &file) in files.iter().enumerate() {
        let _ = writeln!(
            out,
            "{i}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            !program.skip_type_checking(file, false) as u8,
            file.is_declaration_file.get() as u8,
            file.node_count.get(),
            file.text().len(),
            file.imports().len(),
            associations[i],
            file.file_name()
        );
    }
    std::fs::write(format!("{path}.files.tsv"), out).expect("TSRS_ASSIGNMENT_DUMP");
    let file_indices: FxHashMap<P<SourceFile>, usize> = files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    let mut out = String::new();
    for (file_index, file) in files.iter().enumerate() {
        let Some(resolved_modules) = program.resolved_modules.get(file.path()) else {
            continue;
        };
        let mut targets: Vec<usize> = resolved_modules
            .values()
            .filter(|r| r.is_resolved())
            .filter_map(|r| program.get_source_file_for_resolved_module(&r.resolved_file_name))
            .filter_map(|f| file_indices.get(&f).copied())
            .filter(|&t| t != file_index)
            .collect();
        targets.sort_unstable();
        for t in targets {
            let _ = writeln!(out, "{file_index}\t{t}");
        }
    }
    std::fs::write(format!("{path}.edges.tsv"), out).expect("TSRS_ASSIGNMENT_DUMP");
}

// getImportAdjacency returns an undirected import graph represented by file index.
fn get_import_adjacency(program: &Program) -> Vec<Vec<usize>> {
    let files = &program.files;
    let mut file_indices: FxHashMap<P<SourceFile>, usize> = FxHashMap::default();
    for (i, &file) in files.iter().enumerate() {
        file_indices.insert(file, i);
    }
    // The in-program import targets of each file, in resolution-map order (looking a resolved file name up
    // normalizes it, so this part runs on the worker pool); the adjacency lists are then built in file order.
    let targets_of = |file_index: usize| -> Vec<usize> {
        let Some(resolved_modules) = program.resolved_modules.get(files[file_index].path()) else {
            return Vec::new();
        };
        // Go iterates the resolution map in random order; FENNEL only counts neighbors, so the
        // order of entries within an adjacency list does not affect the result.
        resolved_modules
            .values()
            .filter(|resolved| resolved.is_resolved())
            .filter_map(|resolved| program.get_source_file_for_resolved_module(&resolved.resolved_file_name))
            .filter_map(|imported_file| file_indices.get(&imported_file).copied())
            .filter(|&imported_index| imported_index != file_index)
            .collect()
    };
    let targets: Vec<Vec<usize>> = if program.single_threaded() {
        (0..files.len()).map(targets_of).collect()
    } else {
        use rayon::prelude::*;
        crate::program::worker_pool().install(|| (0..files.len()).into_par_iter().map(targets_of).collect())
    };
    let mut adjacent_files: Vec<Vec<usize>> = vec![Vec::new(); files.len()];
    for (file_index, file_targets) in targets.into_iter().enumerate() {
        for imported_index in file_targets {
            adjacent_files[file_index].push(imported_index);
            adjacent_files[imported_index].push(file_index);
        }
    }
    adjacent_files
}

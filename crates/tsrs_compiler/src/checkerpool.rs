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

// Go `context.Context` (see tsrs_checker::Context).
#[cfg(not(feature = "checker"))]
#[derive(Clone, Copy, Default)]
pub struct Context;

#[cfg(not(feature = "checker"))]
impl Checker {
    pub fn get_diagnostics_exported(&mut self, _ctx: Context, _source_file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        unimplemented!("tsrs_compiler was built without the `checker` feature")
    }

    pub fn get_suggestion_diagnostics(&mut self, _ctx: Context, _source_file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        unimplemented!("tsrs_compiler was built without the `checker` feature")
    }

    pub fn get_global_diagnostics(&mut self) -> Vec<P<Diagnostic>> {
        unimplemented!("tsrs_compiler was built without the `checker` feature")
    }
}

#[cfg(not(feature = "checker"))]
fn new_checker(_program: &'static Program) -> Box<Checker> {
    Box::new(Checker { type_count: 0, symbol_count: 0, total_instantiation_count: 0, lazy_member_stats: Default::default() })
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
unsafe impl Send for CheckerSlot {}
unsafe impl Sync for CheckerSlot {}

pub struct CheckerGuard<'a>(MutexGuard<'a, Box<Checker>>);

impl std::ops::Deref for CheckerGuard<'_> {
    type Target = Checker;
    fn deref(&self) -> &Checker {
        &self.0
    }
}

impl std::ops::DerefMut for CheckerGuard<'_> {
    fn deref_mut(&mut self) -> &mut Checker {
        &mut self.0
    }
}

pub(crate) struct poolState {
    checkers: Vec<CheckerSlot>,
    file_associations: FxHashMap<P<SourceFile>, usize>,
    // Checker index per program file index.
    pub(crate) associations: Vec<usize>,
    // TSRS_ASSIGNMENT_STATS only: per for_each_checker_group_do call, (seconds, files run) per checker.
    pub(crate) group_runs: Mutex<Vec<Vec<(f64, usize)>>>,
    // TSRS_FILE_TIMES only: (file, checker, seconds) per checked file, in completion order.
    pub(crate) file_times: Mutex<Vec<(P<SourceFile>, usize, f64)>>,
}

// TSRS_FILE_TIMES=<path> (experiments): after checking, write one line per file run by a checker group:
// checker, seconds, node count, text length, import count, file name, and the file's node-kind histogram
// (`Kind=count` pairs), for fitting assignment cost models offline.
pub(crate) fn file_times_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| std::env::var("TSRS_FILE_TIMES").ok().filter(|v| !v.is_empty())).as_deref()
}

pub(crate) fn write_file_times(program: &'static Program) {
    use std::fmt::Write;
    let (Some(path), Some(state)) = (file_times_path(), program.pool().state.get()) else {
        return;
    };
    fn count_kinds(node: P<tsrs_ast::Node>, counts: &mut [u32]) {
        counts[node.kind as usize] += 1;
        node.for_each_child(&mut |child| {
            count_kinds(child, counts);
            false
        });
    }
    let mut out = String::new();
    for &(file, checker, seconds) in state.file_times.lock().unwrap().iter() {
        let mut counts = vec![0u32; tsrs_ast::Kind::Count as usize + 1];
        count_kinds(file.as_node(), &mut counts);
        let _ = write!(out, "{checker}\t{seconds:.6}\t{}\t{}\t{}\t{}\t", file.node_count.get(), file.text().len(), file.imports().len(), file.file_name());
        for (kind, &count) in counts.iter().enumerate().filter(|(_, &c)| c > 0) {
            let _ = write!(out, "{:?}={count} ", tsrs_ast::Kind::from_i16(kind as i16));
        }
        out.push('\n');
    }
    std::fs::write(path, out).expect("TSRS_FILE_TIMES");
}

// TSRS_ASSIGNMENT_STATS=1: record per-checker group timings and, after `--extendedDiagnostics`, print the
// per-checker assignment report (checkerpool_stats.rs). Read once; nothing is recorded when unset.
pub fn assignment_stats_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TSRS_ASSIGNMENT_STATS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

pub(crate) struct checkerPool {
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
    pub(crate) fn new(program: &Program) -> checkerPool {
        let mut checker_count = 4;
        if program.single_threaded() {
            checker_count = 1;
        } else if let Some(c) = program.options().checkers {
            checker_count = c as usize;
        }

        checker_count = checker_count.min(program.files.len()).min(256).max(1);

        checkerPool { checker_count, single_threaded: program.single_threaded() || checker_count == 1, state: OnceLock::new() }
    }

    fn create_checkers(&self, program: &'static Program) -> &poolState {
        self.state.get_or_init(|| {
            if tsrs_core::ptr::shared_check::enabled() {
                // Debug aid: bind up front so every parser/binder allocation is recorded as shared.
                tsrs_core::ptr::shared_check::thaw();
                program.bind_source_files();
                tsrs_core::ptr::shared_check::freeze_shared_objects();
            }
            let create_start = std::time::Instant::now();
            let slots: Vec<Mutex<Option<CheckerSlot>>> = (0..self.checker_count).map(|_| Mutex::new(None)).collect();
            run_work_group(self.single_threaded, self.checker_count, |i| {
                *slots[i].lock().unwrap() = Some(CheckerSlot(Mutex::new(new_checker(program))));
            });
            let checkers: Vec<CheckerSlot> = slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect();
            tsrs_core::phases::record("Checkers: create", create_start.elapsed());

            let files = &program.files;
            let associations = tsrs_core::phases::time("Checkers: assign files", || compute_associations(program, self.checker_count));
            let mut file_associations = FxHashMap::default();
            for (i, &file) in files.iter().enumerate() {
                file_associations.insert(file, associations[i]);
            }
            poolState { checkers, file_associations, associations, group_runs: Mutex::new(Vec::new()), file_times: Mutex::new(Vec::new()) }
        })
    }

    pub(crate) fn state(&self, program: &'static Program) -> &poolState {
        self.create_checkers(program)
    }

    pub(crate) fn checker_count(&self) -> usize {
        self.checker_count
    }

    // GetChecker: when file is Some, returns the checker associated with that file; otherwise the first checker.
    pub(crate) fn get_checker(&self, program: &'static Program, file: Option<P<SourceFile>>) -> CheckerGuard<'_> {
        let state = self.create_checkers(program);
        let idx = match file {
            Some(file) => state.file_associations[&file],
            None => 0,
        };
        CheckerGuard(state.checkers[idx].0.lock().unwrap())
    }

    // Runs `cb` for each checker in the pool, locking each checker while it runs.
    pub(crate) fn for_each_checker_parallel(&self, program: &'static Program, cb: impl Fn(usize, &mut Checker) + Sync) {
        let state = self.create_checkers(program);
        let run = |idx: usize| {
            let mut guard = state.checkers[idx].0.lock().unwrap();
            cb(idx, &mut guard);
        };
        run_work_group(self.single_threaded, state.checkers.len(), run);
    }

    pub(crate) fn get_global_diagnostics(&self, program: &'static Program) -> Vec<P<Diagnostic>> {
        let state = self.create_checkers(program);
        let global_diagnostics: Vec<Mutex<Vec<P<Diagnostic>>>> = (0..state.checkers.len()).map(|_| Mutex::new(Vec::new())).collect();
        self.for_each_checker_parallel(program, |idx, checker| {
            *global_diagnostics[idx].lock().unwrap() = checker.get_global_diagnostics();
        });
        let all: Vec<P<Diagnostic>> = global_diagnostics.into_iter().flat_map(|d| d.into_inner().unwrap()).collect();
        sort_and_deduplicate_diagnostics(&all)
    }

    // forEachCheckerGroupDo runs one task per checker. Each task iterates the provided files,
    // processing only those assigned to its checker. Within each checker's set, files are
    // visited in their original order.
    pub(crate) fn for_each_checker_group_do(
        &self,
        program: &'static Program,
        files: &[P<SourceFile>],
        single_threaded: bool,
        cb: impl Fn(&mut Checker, usize, P<SourceFile>) + Sync,
    ) {
        let state = self.create_checkers(program);
        let stats = assignment_stats_enabled();
        let times: Vec<Mutex<(f64, usize)>> = if stats { (0..state.checkers.len()).map(|_| Mutex::new((0.0, 0))).collect() } else { Vec::new() };
        let file_times = file_times_path().is_some();
        let run = |checker_idx: usize| {
            let start = stats.then(std::time::Instant::now);
            let mut count = 0;
            let mut guard = state.checkers[checker_idx].0.lock().unwrap();
            for (i, &file) in files.iter().enumerate() {
                if state.file_associations.get(&file) == Some(&checker_idx) {
                    let file_start = file_times.then(std::time::Instant::now);
                    cb(&mut guard, i, file);
                    if let Some(file_start) = file_start {
                        state.file_times.lock().unwrap().push((file, checker_idx, file_start.elapsed().as_secs_f64()));
                    }
                    count += 1;
                }
            }
            if let Some(start) = start {
                *times[checker_idx].lock().unwrap() = (start.elapsed().as_secs_f64(), count);
            }
        };
        run_work_group(single_threaded || self.single_threaded, state.checkers.len(), run);
        if stats {
            state.group_runs.lock().unwrap().push(times.into_iter().map(|t| t.into_inner().unwrap()).collect());
        }
    }
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

// tsrs-only: how files are assigned to checkers. `--checkerAssignment <name>` (CLI) or
// TSRS_CHECKER_ASSIGNMENT=<name> (any binary). The assignment never changes what a file's diagnostics are, only
// which checker computes them (and so how much checker state is duplicated across checkers).
//   locality (default): directory-subtree groups packed onto checkers with Go's FENNEL (below)
//   go:                 Go's createCheckers association (FENNEL over single files in program order)
//   file:<path>:        one checker index per line by program file index (experiments)
pub enum CheckerAssignment {
    Locality,
    Go,
    File(String),
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
        _ => name.strip_prefix("file:").map(|p| CheckerAssignment::File(p.to_string())),
    }
}

fn checker_assignment() -> CheckerAssignment {
    let name = match CLI_CHECKER_ASSIGNMENT.get() {
        Some(name) => name.clone(),
        None => std::env::var("TSRS_CHECKER_ASSIGNMENT").unwrap_or_default(),
    };
    parse_checker_assignment(&name).unwrap_or_else(|| panic!("unknown checker assignment {name:?} (locality, go, file:<path>)"))
}

// A directory subtree whose checked weight is at most 1/LOCALITY_GROUP_FRACTION of an average checker load is
// kept on one checker.
const LOCALITY_GROUP_FRACTION: i64 = 4;
const LOCALITY_PENALTY_MULTIPLIER: i64 = 1;

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
    let total: i64 = order.iter().map(|&i| weights[i]).sum();
    let threshold = total / (checker_count as i64 * LOCALITY_GROUP_FRACTION);

    // Checked weight of every directory subtree (keys are path prefixes ending before a '/').
    let mut subtree_weights: FxHashMap<&str, i64> = FxHashMap::default();
    for &i in &order {
        let path: &str = files[i].path();
        for (pos, _) in path.match_indices('/') {
            *subtree_weights.entry(&path[..pos]).or_default() += weights[i];
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
        group_weights[group] += weights[i];
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

    let mut group_order: Vec<usize> = (0..group_weights.len()).collect();
    group_order.sort_by(|&a, &b| group_weights[b].cmp(&group_weights[a]).then(a.cmp(&b)));
    let group_associations =
        get_checker_associations_in_order(&group_weights, &group_adjacency, Some(&group_order), checker_count, LOCALITY_PENALTY_MULTIPLIER);
    for &i in &order {
        associations[i] = group_associations[group_of_file[i]];
    }
    associations
}

// Go's file weights (base work, regime-2 source multiplier, normalized import fanout) for the files a checker
// processes; 0 for declaration and JSON files that are not type checked (skipLibCheck / skipDefaultLibCheck;
// JSON files are never checked): no checker does work for them. Other source files keep their weight even
// when not type checked (noCheck, JS without checkJs), since declaration diagnostics still use their checker.
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
            if f.is_declaration_file.get() { base } else { base * CHECKER_ASSOCIATION_SOURCE_FILE_WEIGHT_MULTIPLIER }
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

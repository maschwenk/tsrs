use std::sync::{Mutex, MutexGuard, OnceLock};

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::P;

use crate::program::{sort_and_deduplicate_diagnostics, Program};

#[cfg(feature = "checker")]
pub use tsrs_checker::Checker;

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
}

#[cfg(not(feature = "checker"))]
impl Checker {
    pub fn get_diagnostics(&mut self, _source_file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        unimplemented!("tsrs_compiler was built without the `checker` feature")
    }

    pub fn get_global_diagnostics(&mut self) -> Vec<P<Diagnostic>> {
        unimplemented!("tsrs_compiler was built without the `checker` feature")
    }
}

#[cfg(not(feature = "checker"))]
fn new_checker(_program: &'static Program) -> Box<Checker> {
    Box::new(Checker { type_count: 0, symbol_count: 0, total_instantiation_count: 0 })
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

struct poolState {
    checkers: Vec<CheckerSlot>,
    file_associations: FxHashMap<P<SourceFile>, usize>,
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
        // Go defaults to 4 checkers; tsrs defaults to a single checker until parallel checking
        // is turned on. `--checkers N` still selects the Go assignment scheme for N checkers.
        let mut checker_count = 1;
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
            let checkers: Vec<CheckerSlot> = (0..self.checker_count)
                .map(|_| CheckerSlot(Mutex::new(new_checker(program))))
                .collect();

            let files = &program.files;
            let mut associations = vec![0usize; files.len()];
            if self.checker_count > 1 {
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
                let policy = get_checker_association_policy(total_base_weight, declaration_base_weight, self.checker_count);
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
                associations = get_checker_associations_in_order(
                    &file_weights,
                    &adjacent_files,
                    file_order.as_deref(),
                    self.checker_count,
                    policy.balance_penalty_multiplier,
                );
            }
            let mut file_associations = FxHashMap::default();
            for (i, &file) in files.iter().enumerate() {
                file_associations.insert(file, associations[i]);
            }
            poolState { checkers, file_associations }
        })
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
        if self.single_threaded {
            (0..state.checkers.len()).for_each(run);
        } else {
            crate::program::worker_pool().install(|| (0..state.checkers.len()).into_par_iter().for_each(run));
        }
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
        let run = |checker_idx: usize| {
            let mut guard = state.checkers[checker_idx].0.lock().unwrap();
            for (i, &file) in files.iter().enumerate() {
                if state.file_associations.get(&file) == Some(&checker_idx) {
                    cb(&mut guard, i, file);
                }
            }
        };
        if single_threaded || self.single_threaded {
            (0..state.checkers.len()).for_each(run);
        } else {
            crate::program::worker_pool().install(|| (0..state.checkers.len()).into_par_iter().for_each(run));
        }
    }
}

// getImportAdjacency returns an undirected import graph represented by file index.
fn get_import_adjacency(program: &Program) -> Vec<Vec<usize>> {
    let files = &program.files;
    let mut file_indices: FxHashMap<P<SourceFile>, usize> = FxHashMap::default();
    for (i, &file) in files.iter().enumerate() {
        file_indices.insert(file, i);
    }
    let mut adjacent_files: Vec<Vec<usize>> = vec![Vec::new(); files.len()];
    for (file_index, file) in files.iter().enumerate() {
        let Some(resolved_modules) = program.resolved_modules.get(file.path()) else {
            continue;
        };
        // Go iterates the resolution map in random order; FENNEL only counts neighbors, so the
        // order of entries within an adjacency list does not affect the result.
        for resolved in resolved_modules.values() {
            if !resolved.is_resolved() {
                continue;
            }
            let Some(imported_file) = program.get_source_file_for_resolved_module(&resolved.resolved_file_name) else {
                continue;
            };
            let Some(&imported_index) = file_indices.get(&imported_file) else {
                continue;
            };
            if imported_index == file_index {
                continue;
            }
            adjacent_files[file_index].push(imported_index);
            adjacent_files[imported_index].push(file_index);
        }
    }
    adjacent_files
}

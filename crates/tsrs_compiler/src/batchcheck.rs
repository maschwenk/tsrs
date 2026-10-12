//! CLI-only checking: one program-owned shared data store and persistent evaluation contexts scheduled by Rayon.
//! Contexts are borrowed exclusively for a whole job. Recursive checking stays inside that job; it never obtains
//! another context or schedules nested checker work. The external CheckerPool lease API is not used here.

use std::sync::{Arc, Mutex, OnceLock};

use rayon::prelude::*;
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::P;

use crate::checkerpool::{self, Checker, Context, FileQueue, PooledChecker, SplitPlan};
use crate::program::{sort_and_deduplicate_diagnostics, Program};

/// The CLI opts in explicitly. Unsupported modes retain their existing checker ownership and lifetime rules,
/// including when the internal selector asks for the shared engine.
pub(crate) fn enabled(program: &Program) -> bool {
    let requested = match std::env::var("TSRS_CHECK_ENGINE").as_deref() {
        Ok("legacy") => false,
        Ok("shared") | Err(_) => true,
        Ok(value) => panic!("unknown TSRS_CHECK_ENGINE {value:?} (shared or legacy)"),
    };
    let options = program.options();
    requested
        && options.no_emit.is_true()
        && !options.is_incremental()
        && !options.get_emit_declarations()
        && !options.explain_files.is_true()
        && checkerpool::stealing_enabled()
        && checkerpool::max_memory() == 0
        && !matches!(std::env::var("TSRS_SHARED_GRAPH").as_deref(), Ok("1" | "on"))
        && !checkerpool::assignment_stats_enabled()
        && checkerpool::file_times_path().is_none()
        && checkerpool::checker_cost_cache_path().is_none()
}

pub(crate) struct BatchCheck {
    program: &'static Program,
    count: usize,
    executor: OnceLock<rayon::ThreadPool>,
    state: OnceLock<Mutex<BatchState>>,
}

struct BatchState {
    contexts: Vec<BatchContext>,
    owners: Vec<usize>,
    weights: Vec<i64>,
    // Diagnostics and shared payloads remain valid even after all evaluation contexts are released.
    shared: Arc<tsrs_checker::SharedCheckData>,
}

// The context's allocations follow its logical owner, even if a later Rayon job runs on another worker.
// CLI programs live until process exit. Their context arenas follow that lifetime, retaining the thread arena's
// destructor-free allocations and exact reuse even when the context moves between Rayon workers.
struct BatchContext {
    context: PooledChecker,
    region: tsrs_core::arena::Region,
}

impl BatchCheck {
    pub(crate) fn new(program: &'static Program) -> Self {
        Self { program, count: checkerpool::checker_count_for_program(program), executor: OnceLock::new(), state: OnceLock::new() }
    }

    pub(crate) fn count(&self) -> usize {
        self.count
    }

    fn executor(&self) -> &rayon::ThreadPool {
        self.executor.get_or_init(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(self.count)
                .stack_size(checkerpool::CHECKER_STACK_SIZE)
                .thread_name(|i| format!("check-context-{i}"))
                .start_handler(|_| tsrs_ast::use_id_blocks())
                .exit_handler(|_| tsrs_core::ptr::release_own_arena())
                .build()
                .expect("failed to create batch checker executor")
        })
    }

    fn state(&self) -> &Mutex<BatchState> {
        self.state.get_or_init(|| {
            if tsrs_core::ptr::shared_check::enabled() {
                tsrs_core::ptr::shared_check::thaw();
                self.program.bind_source_files();
                tsrs_core::ptr::shared_check::freeze_shared_objects();
            }
            tsrs_core::memsplit::report("parse end");
            if self.count > 1 {
                crate::fileregions::force_shared_lists(self.program);
                // Parsing has finished. Reuse its arena tails on the checking executor, as the legacy backend does.
                crate::program::worker_pool().broadcast(|_| tsrs_core::ptr::release_own_arena());
            }
            crate::fileregions::prepare(self.program);
            let shared = tsrs_checker::SharedCheckData::new(self.program);
            let contexts = tsrs_core::phases::time("Checkers: create", || {
                tsrs_checker::links::set_multiple_checkers(self.count > 1);
                let create = |_| {
                    let region = tsrs_core::arena::Region::clone(tsrs_core::arena::Region::new_permanent_scratch(1 << 20));
                    let context = {
                        let _scope = region.enter_scratch();
                        PooledChecker::new(tsrs_checker::new_checker_with_shared_data(Arc::clone(&shared)))
                    };
                    BatchContext { context, region }
                };
                if self.count == 1 {
                    (0..self.count).map(create).collect()
                } else {
                    self.executor().install(|| (0..self.count).into_par_iter().map(create).collect())
                }
            });
            let owners = tsrs_core::phases::time("Checkers: assign files", || checkerpool::compute_associations(self.program, self.count));
            let weights = checkerpool::checked_file_weights(self.program);
            tsrs_core::phases::count("Check contexts", self.count as u64);
            Mutex::new(BatchState { contexts, owners, weights, shared })
        })
    }

    fn run_contexts(&self, contexts: &mut [BatchContext], trim: bool, cb: impl Fn(usize, &mut Checker) + Sync) {
        let run = |(i, context): (usize, &mut BatchContext)| {
            let _scope = context.region.enter_scratch();
            cb(i, &mut context.context);
            if trim {
                context.region.trim();
            }
        };
        if self.count == 1 {
            contexts.iter_mut().enumerate().for_each(run);
        } else {
            self.executor().install(|| contexts.par_iter_mut().enumerate().for_each(run));
        }
    }

    pub(crate) fn for_each_context(&self, cb: impl Fn(usize, &mut Checker) + Sync) {
        let mut state = self.state().lock().unwrap();
        self.run_contexts(&mut state.contexts, false, cb);
    }

    pub(crate) fn global_diagnostics(&self) -> Vec<P<Diagnostic>> {
        let diagnostics = Mutex::new(Vec::new());
        self.for_each_context(|_, context| {
            let globals = context.get_global_diagnostics();
            diagnostics.lock().unwrap().extend(globals);
        });
        sort_and_deduplicate_diagnostics(&diagnostics.into_inner().unwrap())
    }

    /// The one full-program type-check pass. The current file assignment, heavy-file order and work claims are
    /// reused, so changing the executor does not invent another scheduling policy or another recursion history.
    pub(crate) fn check(&self, ctx: &Context, collect: impl Fn(&mut Checker, usize, P<SourceFile>) + Sync) {
        let mut state = self.state().lock().unwrap();
        let files = self.program.files;
        let mut positions = vec![Vec::new(); self.count];
        for i in checkerpool::visit_order(files.len()) {
            positions[state.owners[i]].push(i as u32);
        }
        let active: Vec<_> = (0..self.count).filter(|&i| !positions[i].is_empty()).collect();
        let steal = self.count > 1 && active.len() > 1 && checkerpool::stealing_enabled();
        let sticky = steal && crate::affinity::applies(self.count, files.len()) && checkerpool::steal_sticky();
        let max_thieves = checkerpool::max_thieves_per_queue(self.count);
        let file_weight = |i: u32| state.weights[i as usize].max(0) as u64;
        let split = if steal && crate::splitcheck::split_config().enabled {
            checkerpool::plan_splits(self.program, files, &mut positions, &active, file_weight)
        } else {
            SplitPlan::default()
        };
        let weight = |i: u32| split.item_weight(files.len(), i).unwrap_or_else(|| file_weight(i));
        if steal {
            let total: u64 = positions.iter().flatten().map(|&i| weight(i)).sum();
            let threshold = total / (active.len() as u64 * checkerpool::heavy_share_divisor());
            positions.iter_mut().for_each(|p| checkerpool::heavy_files_first(p, threshold, weight));
        }
        let queues: Vec<_> = positions.into_iter().map(|p| FileQueue::new(p, weight)).collect();
        self.run_contexts(&mut state.contexts, true, |i, context| {
            // The legacy pass starts jobs only for contexts that initially own files. Idle contexts must not
            // become additional thieves, since that would expand the number of independent checking histories.
            if active.binary_search(&i).is_err() {
                return;
            }
            let (mut slot, mut last_victim) = (usize::MAX, usize::MAX);
            while let Some((item, _)) = checkerpool::queues_next(&queues, i, steal, max_thieves, &mut slot, sticky.then_some(&mut last_victim)) {
                if let Some(&(s, piece, _)) = item.checked_sub(files.len()).map(|p| &split.piece_items[p]) {
                    split.files[s].run_queued_piece(context, ctx, piece);
                    continue;
                }
                let file = files[item];
                let split_file = split.split_of.get(&(item as u32)).map(|&s| &split.files[s]);
                if let Some(split_file) = split_file {
                    split_file.run_owner(context, ctx);
                }
                collect(context, item, file);
                if let Some(split_file) = split_file.filter(|f| f.is_shadow()) {
                    split_file.shadow_compare(&context.file_diagnostics_so_far(file));
                }
            }
            if self.count > 1 {
                tsrs_core::arena::trim_own_arena_tail();
            }
        });
        crate::splitcheck::SplitFile::report_stats(&split.files);
        // Context jobs have joined. These are retained arena bytes, not cumulative allocation or process RSS.
        let local_bytes: usize = state.contexts.iter().map(|c| c.region.used_bytes()).sum();
        let local_capacity: usize = state.contexts.iter().map(|c| c.region.allocated_bytes()).sum();
        tsrs_core::phases::count("Check contexts: retained arena bytes", local_bytes as u64);
        tsrs_core::phases::count("Check contexts: arena capacity bytes", local_capacity as u64);
        tsrs_core::phases::count("Check contexts: drop entries", state.contexts.iter().map(|c| c.region.drop_entries() as u64).sum());
        tsrs_core::phases::count("Check contexts: drop list bytes", state.contexts.iter().map(|c| c.region.drop_list_bytes() as u64).sum());
        let shared = state.shared.payloads.stats();
        tsrs_core::phases::count("Shared payloads: requested bytes", shared.requested_bytes);
        tsrs_core::phases::count("Shared payloads: stored bytes", shared.stored_bytes);
        tsrs_core::phases::count("Shared payloads: avoided bytes", shared.requested_bytes.saturating_sub(shared.stored_bytes));
        tsrs_core::phases::count("Shared payloads: retained arena bytes", shared.arena_used_bytes);
        tsrs_core::phases::count("Shared payloads: arena capacity bytes", shared.arena_allocated_bytes);
        tsrs_core::phases::count("Shared payloads: interning table bytes", shared.table_bytes);
        tsrs_core::phases::count("Shared payloads: hits", shared.hits);
        tsrs_core::phases::count("Shared payloads: misses", shared.misses);
        if tsrs_core::memsplit::enabled() {
            if self.count == 1 {
                tsrs_core::memsplit::note_own_stack();
            } else {
                self.executor().broadcast(|_| tsrs_core::memsplit::note_own_stack());
            }
            // Every context job has joined; no arena is being written while residency is inspected.
            tsrs_core::memsplit::report("check end");
            if tsrs_core::memsplit::purge_after_check() {
                tsrs_core::memsplit::heap_collect(true);
                tsrs_core::memsplit::report("check end, after mi_collect(true)");
            }
        }
    }
}

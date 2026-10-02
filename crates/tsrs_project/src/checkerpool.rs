use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_compiler::{sort_and_deduplicate_diagnostics, Checker, CheckerHandle, CheckerPool, PooledChecker, Program};
use tsrs_core::context::{get_checker_lifetime, get_request_id, CheckerLifetime, Context};
use tsrs_core::arena::{Region, RegionScope};
use tsrs_core::P;

use crate::background::{after_func, Timer};

// checkerHeldAnonymous is a sentinel stored in heldBy when a checker is held
// by a caller that has no request ID (e.g., context.Background()). This
// distinguishes "held without ID" from "not held" (empty string).
const checkerHeldAnonymous: &str = "<anonymous>";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CheckerPoolOptions {
    // MaxCheckers controls the total number of checker slots per project
    // (1 dedicated diagnostics checker + N-1 query checkers). Minimum 2.
    // Zero uses the default (4).
    pub max_checkers: usize,
    // IdleTimeout controls how long an idle checker is kept
    // before being disposed. Zero uses the default (30s).
    pub idle_timeout: Duration,
}

// Go's buffered channel used as a counting semaphore (`sem <- struct{}{}` / `<-sem`).
struct semaphore {
    used: Mutex<usize>,
    capacity: usize,
    freed: Condvar,
}

impl semaphore {
    fn new(capacity: usize) -> semaphore {
        semaphore { used: Mutex::new(0), capacity, freed: Condvar::new() }
    }

    fn acquire(&self) {
        let mut used = self.used.lock().unwrap();
        while *used >= self.capacity {
            used = self.freed.wait(used).unwrap();
        }
        *used += 1;
    }

    fn release(&self) {
        *self.used.lock().unwrap() -= 1;
        self.freed.notify_one();
    }
}

// checkerPool manages three categories of type checkers for a project:
//
//   - Diagnostics (index 0): A single checker for LSP diagnostics, providing
//     consistent walk order. Idle-cleaned.
//   - Temporary (indices 1+): Ephemeral query checkers for LSP operations.
//     Idle-cleaned after a configurable timeout.
//   - API: A single checker for API operations, providing stable
//     instance identity for reference equality on type/symbol handles.
//     Never idle-cleaned.
pub struct checkerPool {
    opts: CheckerPoolOptions,
    program: &'static Program,

    mu: Mutex<checkerPoolState>,

    diag_sem: semaphore,
    query_sem: semaphore,
    persistent_sem: semaphore,

    log: Box<dyn Fn(&str) + Send + Sync>,
    self_ref: Weak<checkerPool>,

    // Memory regions (docs/LSP.md "Memory plan for a long-lived server"), not in Go. Each checker allocates in its
    // own region (keyed by the checker's address): while it is created and while it is held, the region is the
    // holding thread's allocation target. Disposed checkers are parked here with their regions, because data they
    // made can still be referenced from pool- or program-lifetime structures (global diagnostics, the program's
    // declaration diagnostics cache); checkers and regions are freed with the pool (`free_checkers`, called when the
    // program is freed). Declared last: dropped after the checkers in `mu`.
    parked: Mutex<Vec<PooledChecker>>,
    regions: Mutex<FxHashMap<usize, Region>>,
}

struct checkerPoolState {
    // discarded is set when the pool's program has been replaced. The pool
    // remains fully functional but stops its idle-cleanup timer so that
    // query checkers are not disposed until the pool is GC'd.
    discarded: bool,

    // checkers[0] is the diagnostics checker.
    // checkers[1:] are ephemeral query checkers.
    // All are idle-cleaned.
    checkers: Vec<Option<PooledChecker>>,
    held_by: Vec<String>, // heldBy[i] is the requestID holding checker i, checkerHeldAnonymous, or "" if not held
    file_associations: FxHashMap<P<SourceFile>, usize>, // file → query checker index (1+)
    request_associations: FxHashMap<String, usize>,    // requestID → checker index

    // lastReleased tracks when each checker was last released.
    last_released: Vec<Option<Instant>>,

    // cleanupTimer is reset each time a checker is released.
    // When it fires, idle checkers are disposed.
    cleanup_timer: Option<Timer>,

    // persistentChecker is the API checker. It is never idle-cleaned,
    // providing stable instance identity for API clients.
    persistent_checker: Option<PooledChecker>,
    persistent_held: bool,

    global_diag_accumulated: Vec<P<Diagnostic>>,
    global_diag_changed: bool,
    global_diag_checker_count: Vec<usize>, // per-checker count of globals last seen
}

// checkerpool.go:84
pub(crate) fn new_checker_pool(mut opts: CheckerPoolOptions, program: &'static Program, log: Option<Box<dyn Fn(&str) + Send + Sync>>) -> Arc<checkerPool> {
    if opts.max_checkers == 0 {
        opts.max_checkers = 4;
    } else if opts.max_checkers < 2 {
        opts.max_checkers = 2; // at least 1 diagnostics + 1 query checker
    }
    if opts.idle_timeout.is_zero() {
        opts.idle_timeout = Duration::from_secs(30);
    }
    let query_slots = opts.max_checkers - 1;
    let log = log.unwrap_or_else(|| Box::new(|_msg: &str| {}));
    Arc::new_cyclic(|self_ref| checkerPool {
        program,
        opts,
        mu: Mutex::new(checkerPoolState {
            discarded: false,
            checkers: (0..opts.max_checkers).map(|_| None).collect(),
            held_by: vec![String::new(); opts.max_checkers],
            file_associations: FxHashMap::default(),
            request_associations: FxHashMap::default(),
            last_released: vec![None; opts.max_checkers],
            cleanup_timer: None,
            persistent_checker: None,
            persistent_held: false,
            global_diag_accumulated: Vec::new(),
            global_diag_changed: false,
            global_diag_checker_count: vec![0; opts.max_checkers],
        }),
        diag_sem: semaphore::new(1),
        query_sem: semaphore::new(query_slots),
        persistent_sem: semaphore::new(1),
        log,
        self_ref: self_ref.clone(),
        parked: Mutex::new(Vec::new()),
        regions: Mutex::new(FxHashMap::default()),
    })
}

// checkerpool.go:116
// holdTag returns the value to store in heldBy for the given request ID.
fn hold_tag(request_id: &str) -> String {
    if request_id.is_empty() {
        return checkerHeldAnonymous.to_string();
    }
    request_id.to_string()
}


// The `CheckerPool` the program calls (Go: `*checkerPool` itself).
pub(crate) struct checkerPoolHandle(pub(crate) Arc<checkerPool>);

impl CheckerPool for checkerPoolHandle {
    fn get_checker(&self, ctx: &Context, file: Option<P<SourceFile>>) -> CheckerHandle {
        self.0.get_checker(ctx, file)
    }
}

impl checkerPool {
    // Region hook: a new checker is created inside its own region.
    fn new_pooled_checker(&self) -> PooledChecker {
        let region = Region::new(1 << 20);
        let mut checker = {
            let _scope = region.enter();
            // Census builds: the checker struct is built on the stack with unset fields (e.g. the length word of a
            // `None` slice); clear the stale words they would copy.
            tsrs_core::census_scrub_stack();
            PooledChecker::new(tsrs_checker::new_checker(self.program))
        };
        self.regions.lock().unwrap().insert(checker.as_non_null().as_ptr() as usize, region);
        checker
    }

    // Region hook: while a checker is held, its region is the holder's allocation target.
    fn enter_checker_region(&self, c: std::ptr::NonNull<Checker>) -> Option<RegionScope> {
        let region = self.regions.lock().unwrap().get(&(c.as_ptr() as usize)).cloned();
        region.map(|r| r.enter())
    }

    // Region hook: frees every checker of the pool and their regions. Called when the program is freed (nothing
    // holds a checker or refers to checker data any more).
    pub(crate) fn free_checkers(&self) {
        let mut checkers: Vec<PooledChecker> = std::mem::take(&mut *self.parked.lock().unwrap());
        {
            let mut st = self.mu.lock().unwrap();
            checkers.extend(st.checkers.iter_mut().filter_map(Option::take));
            checkers.extend(st.persistent_checker.take());
            st.global_diag_accumulated.clear();
        }
        drop(checkers);
        let regions = std::mem::take(&mut *self.regions.lock().unwrap());
        drop(regions);
    }

    fn arc(&self) -> Arc<checkerPool> {
        self.self_ref.upgrade().expect("checker pool used after it was dropped")
    }

    // checkerpool.go:123
    pub fn get_checker(&self, ctx: &Context, file: Option<P<SourceFile>>) -> CheckerHandle {
        let lifetime = get_checker_lifetime(ctx);
        let mut request_id = get_request_id(ctx).to_string();

        // Request affinity is cleaned up via context.AfterFunc when the request
        // context is done. If the context can never be canceled (ctx.Done() == nil,
        // e.g. context.Background()), that cleanup would never run and
        // requestAssociations would grow unboundedly, so disable affinity entirely.
        if ctx.done_is_nil() {
            request_id = String::new();
        }

        match lifetime {
            CheckerLifetime::Diagnostics => self.get_diagnostics_checker(ctx, &request_id),
            CheckerLifetime::API => self.get_persistent_checker(),
            _ => self.get_query_checker(ctx, &request_id, file),
        }
    }

    // checkerpool.go:158
    // tryReacquireForRequest claims a semaphore slot, then checks whether the given
    // request has an idle associated checker. The caller must provide the
    // appropriate semaphore channel and indicate whether this is a diagnostics
    // request (isDiag). If the associated checker is in the wrong category
    // (e.g. a diagnostics index for a query request), the association is deleted
    // and normal acquisition proceeds.
    //
    // Request affinity is only a preference for an idle checker, not permission to
    // reuse a held checker: concurrent acquisitions can share the same request ID.
    // Returns (checker, release, true) if the checker was reclaimed.
    // Returns (nil, nil, false) if the caller must proceed with
    // normal acquisition — in this case, a semaphore slot has already been claimed.
    // Must NOT be called with p.mu held.
    fn try_reacquire_for_request(&self, request_id: &str, sem: &semaphore, is_diag: bool) -> Option<CheckerHandle> {
        sem.acquire();
        if request_id.is_empty() {
            return None;
        }

        let mut st = self.mu.lock().unwrap();
        let index = *st.request_associations.get(request_id)?;

        // Validate that the associated index matches the expected category.
        // Index 0 is for diagnostics; indices 1+ are for queries.
        if (is_diag && index != 0) || (!is_diag && index == 0) {
            st.request_associations.remove(request_id);
            return None;
        }

        if st.checkers[index].is_none() {
            st.request_associations.remove(request_id);
            return None;
        }

        if st.held_by[index].is_empty() {
            st.held_by[index] = request_id.to_string();
            return Some(self.create_release(&mut st, request_id, index));
        }

        None
    }

    // checkerpool.go:194
    // getDiagnosticsChecker returns the dedicated diagnostics checker (index 0).
    // Creates it on first use. Blocks on diagSem if it's currently in use.
    fn get_diagnostics_checker(&self, ctx: &Context, request_id: &str) -> CheckerHandle {
        const diagIndex: usize = 0;

        if let Some(handle) = self.try_reacquire_for_request(request_id, &self.diag_sem, true) {
            return handle;
        }

        // Token consumed — proceed with normal acquisition.
        let mut st = self.mu.lock().unwrap();

        if st.checkers[diagIndex].is_none() {
            (self.log)("checkerpool: Creating diagnostics checker");
            st.checkers[diagIndex] = Some(self.new_pooled_checker());
        }

        st.held_by[diagIndex] = hold_tag(request_id);
        (self.log)(&format!("checkerpool: Acquired diagnostics checker for request {}", hold_tag(request_id)));
        if !request_id.is_empty() && !st.request_associations.contains_key(request_id) {
            st.request_associations.insert(request_id.to_string(), diagIndex);
            self.register_request_cleanup(ctx, request_id);
        }
        self.create_release(&mut st, request_id, diagIndex)
    }

    // checkerpool.go:226
    // getQueryChecker returns an ephemeral query checker from indices 1+.
    // Uses request affinity, then file affinity, then finds/creates.
    // Blocks on querySem if all query slots are in use.
    fn get_query_checker(&self, ctx: &Context, request_id: &str, file: Option<P<SourceFile>>) -> CheckerHandle {
        if let Some(handle) = self.try_reacquire_for_request(request_id, &self.query_sem, false) {
            return handle;
        }

        // Token consumed — proceed with normal acquisition.
        let mut st = self.mu.lock().unwrap();

        // Try file affinity.
        if let Some(file) = file {
            if let Some(&index) = st.file_associations.get(&file) {
                if index > 0 && st.checkers[index].is_some() && st.held_by[index].is_empty() {
                    st.held_by[index] = hold_tag(request_id);
                    if !request_id.is_empty() && !st.request_associations.contains_key(request_id) {
                        st.request_associations.insert(request_id.to_string(), index);
                        self.register_request_cleanup(ctx, request_id);
                    }
                    return self.create_release(&mut st, request_id, index);
                }
            }
        }

        // Find any available query checker or create one.
        let index = self.find_or_create_query_checker_locked(&mut st);
        st.held_by[index] = hold_tag(request_id);
        (self.log)(&format!("checkerpool: Acquired query checker {} for request {}", index, hold_tag(request_id)));
        if !request_id.is_empty() && !st.request_associations.contains_key(request_id) {
            st.request_associations.insert(request_id.to_string(), index);
            self.register_request_cleanup(ctx, request_id);
        }
        if let Some(file) = file {
            st.file_associations.insert(file, index);
        }
        self.create_release(&mut st, request_id, index)
    }

    // checkerpool.go:270
    // findOrCreateQueryCheckerLocked returns an idle query checker or creates one
    // in the first empty slot. The semaphore guarantees at least one slot is
    // available. Must be called with p.mu held.
    fn find_or_create_query_checker_locked(&self, st: &mut checkerPoolState) -> usize {
        // Prefer an existing idle checker.
        for i in 1..st.checkers.len() {
            if st.checkers[i].is_some() && st.held_by[i].is_empty() {
                return i;
            }
        }
        // Create in the first empty slot.
        for i in 1..st.checkers.len() {
            if st.checkers[i].is_none() {
                (self.log)(&format!("checkerpool: Creating query checker {}", i));
                st.checkers[i] = Some(self.new_pooled_checker());
                return i;
            }
        }
        panic!("checkerpool: no available query slot despite holding semaphore token");
    }

    // checkerpool.go:289
    fn get_persistent_checker(&self) -> CheckerHandle {
        self.persistent_sem.acquire();
        let mut st = self.mu.lock().unwrap();

        if st.persistent_checker.is_none() {
            (self.log)("checkerpool: Creating persistent checker");
            st.persistent_checker = Some(self.new_pooled_checker());
        }

        let c = st.persistent_checker.as_mut().unwrap().as_non_null();
        st.persistent_held = true;
        drop(st);

        let region_scope = self.enter_checker_region(c);
        let p = self.arc();
        let release = move || {
            drop(region_scope);
            let mut st = p.mu.lock().unwrap();
            st.persistent_held = false;
            let canceled = st.persistent_checker.as_mut().is_some_and(|pc| pc.as_non_null() == c && pc.was_canceled());
            if canceled {
                // A canceled checker panics on reuse, so drop it; the next API
                // acquisition will create a fresh persistent checker.
                (p.log)("checkerpool: Persistent checker was canceled, disposing");
                if let Some(old) = st.persistent_checker.take() {
                    p.dispose_checker(old);
                }
            }
            drop(st);
            p.persistent_sem.release();
        };
        // SAFETY: the persistent checker is held (`persistentHeld`, one semaphore slot) until `release` runs; the
        // pool keeps it alive in `persistent_checker` (or leaks it when disposed).
        unsafe { CheckerHandle::from_raw(c, release) }
    }

    // checkerpool.go:319
    fn create_release(&self, st: &mut checkerPoolState, request_id: &str, index: usize) -> CheckerHandle {
        let c = st.checkers[index].as_mut().unwrap().as_non_null();
        let region_scope = self.enter_checker_region(c);
        let p = self.arc();
        let request_id = request_id.to_string();
        let release = move || {
            drop(region_scope);
            let mut st = p.mu.lock().unwrap();

            let was_canceled = st.checkers[index].as_mut().unwrap().was_canceled();
            if was_canceled {
                // Canceled checkers must be disposed.
                (p.log)(&format!("checkerpool: Checker {} for request {} was canceled, disposing", index, hold_tag(&request_id)));
                p.dispose_checker_locked(&mut st, index, c);
            } else {
                // Query checkers can produce incidental errors while serializing types.
                if index == 0 {
                    p.merge_global_diagnostics_from_checker_locked(&mut st, index);
                }
                st.held_by[index] = String::new();
                st.last_released[index] = Some(Instant::now());
                if !st.discarded {
                    p.schedule_cleanup_locked(&mut st);
                }
                // If discarded, skip scheduling cleanup — checkers stay alive
                // until the pool is garbage collected so that API clients can
                // continue resolving type/symbol handles.
            }

            // Unlock before releasing the semaphore slot. If we received from
            // the channel while holding p.mu, a woken goroutine could immediately
            // try to acquire p.mu, risking priority inversion or unnecessary
            // contention.
            drop(st);

            // Release the semaphore slot.
            if index == 0 {
                p.diag_sem.release();
            } else {
                p.query_sem.release();
            }
        };
        // SAFETY: checker `index` is marked held (`heldBy[index]`) until `release` runs; held checkers are never
        // disposed by anyone else, and a disposed checker is leaked, so the pointer stays valid.
        unsafe { CheckerHandle::from_raw(c, release) }
    }

    // checkerpool.go:361
    // registerRequestCleanup uses context.AfterFunc to delete the request
    // association when the request context is done. This prevents the map
    // from growing unboundedly with completed request IDs.
    // Must be called with p.mu held; the cleanup runs asynchronously.
    fn register_request_cleanup(&self, ctx: &Context, request_id: &str) {
        let p = self.self_ref.clone();
        let request_id = request_id.to_string();
        ctx.after_func(move || {
            if let Some(p) = p.upgrade() {
                p.mu.lock().unwrap().request_associations.remove(&request_id);
            }
        });
    }

    // checkerpool.go:373
    // scheduleCleanupLocked resets (or starts) the cleanup timer so it fires at
    // the earliest pending checker-expiration deadline among all currently idle,
    // unheld checkers.
    // Must be called with p.mu held. Must NOT be called on discarded pools.
    fn schedule_cleanup_locked(&self, st: &mut checkerPoolState) {
        let mut earliest_deadline: Option<Instant> = None;
        for i in 0..st.checkers.len() {
            let Some(last_released) = st.last_released[i] else {
                continue;
            };
            if st.checkers[i].is_none() || !st.held_by[i].is_empty() {
                continue;
            }
            let deadline = last_released + self.opts.idle_timeout;
            if earliest_deadline.is_none_or(|earliest| deadline < earliest) {
                earliest_deadline = Some(deadline);
            }
        }
        let Some(earliest_deadline) = earliest_deadline else {
            // No idle checkers remain — stop the timer if it exists.
            if let Some(timer) = st.cleanup_timer.take() {
                timer.stop();
            }
            return;
        };
        let mut delay = earliest_deadline.saturating_duration_since(Instant::now());
        if delay.is_zero() {
            delay = Duration::from_millis(1);
        }
        if let Some(timer) = &st.cleanup_timer {
            timer.reset(delay);
        } else {
            let p = self.self_ref.clone();
            st.cleanup_timer = Some(after_func(delay, move || {
                if let Some(p) = p.upgrade() {
                    p.cleanup_idle_checkers();
                }
            }));
        }
    }

    // checkerpool.go:405
    // cleanupIdleCheckers disposes checkers that have been idle for longer than
    // the idle timeout. The API checker is separate and never idle-cleaned.
    fn cleanup_idle_checkers(&self) {
        let mut st = self.mu.lock().unwrap();
        // The timer callback may already have been in flight when Discard() called
        // Stop() (which does not guarantee the callback won't run). Bail out without
        // rescheduling so a discarded pool doesn't keep itself alive via a new timer.
        if st.discarded {
            return;
        }
        let now = Instant::now();
        for i in 0..st.checkers.len() {
            if st.checkers[i].is_none() || !st.held_by[i].is_empty() {
                continue;
            }
            let Some(last_released) = st.last_released[i] else {
                continue;
            };
            let idle = now.duration_since(last_released);
            if idle >= self.opts.idle_timeout {
                (self.log)(&format!("checkerpool: Disposing idle checker {} (idle {:?})", i, idle));
                let c = st.checkers[i].as_mut().unwrap().as_non_null();
                self.dispose_checker_locked(&mut st, i, c);
            }
        }
        // Reschedule for any remaining idle-but-not-yet-expired checkers.
        // scheduleCleanupLocked will Reset the existing timer rather than
        // creating a new one, avoiding goroutine leaks.
        self.schedule_cleanup_locked(&mut st);
    }

    // checkerpool.go:437
    // disposeCheckerLocked removes a checker from the pool and clears all associations
    // (file and request) that reference it. Must be called with p.mu held.
    fn dispose_checker_locked(&self, st: &mut checkerPoolState, index: usize, c: std::ptr::NonNull<Checker>) {
        assert!(st.checkers[index].as_mut().is_some_and(|pc| pc.as_non_null() == c));
        if let Some(old) = st.checkers[index].take() {
            self.dispose_checker(old);
        }
        st.held_by[index] = String::new();
        st.global_diag_checker_count[index] = 0;
        st.last_released[index] = None;
        st.file_associations.retain(|_, idx| *idx != index);
        st.request_associations.retain(|_, idx| *idx != index);
    }

    // checkerpool.go:458
    // mergeGlobalDiagnosticsFromCheckerLocked checks if the given checker has produced new global
    // diagnostics since the last time we looked, and if so merges them into the accumulated set.
    // Must be called with p.mu held.
    fn merge_global_diagnostics_from_checker_locked(&self, st: &mut checkerPoolState, index: usize) {
        let globals = st.checkers[index].as_mut().unwrap().get_global_diagnostics();
        if globals.len() == st.global_diag_checker_count[index] {
            return;
        }
        st.global_diag_checker_count[index] = globals.len();
        let before = st.global_diag_accumulated.len();
        let mut all = std::mem::take(&mut st.global_diag_accumulated);
        all.extend(globals);
        st.global_diag_accumulated = sort_and_deduplicate_diagnostics(&all);
        if st.global_diag_accumulated.len() != before {
            st.global_diag_changed = true;
        }
    }

    // checkerpool.go:473
    // GetGlobalDiagnostics returns the global diagnostics accumulated from the dedicated
    // diagnostics checker across its instances during this pool's lifetime.
    pub fn get_global_diagnostics(&self) -> Vec<P<Diagnostic>> {
        self.mu.lock().unwrap().global_diag_accumulated.clone()
    }

    // checkerpool.go:481
    // TakeNewGlobalDiagnostics reports whether new global diagnostics have been
    // accumulated since the last call, and resets the flag.
    pub fn take_new_global_diagnostics(&self) -> bool {
        let mut st = self.mu.lock().unwrap();
        std::mem::replace(&mut st.global_diag_changed, false)
    }

    // checkerpool.go:493
    // Discard signals that this pool's program has been replaced. The pool
    // remains functional but stops its idle-cleanup timer so that checkers
    // are not disposed until the pool is GC'd. The API checker is unaffected
    // since it is never idle-cleaned.
    pub fn discard(&self) {
        let mut st = self.mu.lock().unwrap();
        if st.discarded {
            return; // already discarded
        }
        (self.log)("checkerpool: Discarding pool, stopping idle cleanup");
        st.discarded = true;
        if let Some(timer) = st.cleanup_timer.take() {
            timer.stop();
        }
    }

    #[cfg(test)]
    pub(crate) fn checker_ptr(&self, index: usize) -> Option<*const Checker> {
        self.mu.lock().unwrap().checkers[index].as_ref().map(|c| &**c as *const Checker)
    }

    #[cfg(test)]
    pub(crate) fn persistent_ptr(&self) -> Option<*const Checker> {
        self.mu.lock().unwrap().persistent_checker.as_ref().map(|c| &**c as *const Checker)
    }

    #[cfg(test)]
    pub(crate) fn opts(&self) -> CheckerPoolOptions {
        self.opts
    }

    #[cfg(test)]
    pub(crate) fn query_sem_used(&self) -> usize {
        *self.query_sem.used.lock().unwrap()
    }

    #[cfg(test)]
    pub(crate) fn test_state<R>(&self, f: impl FnOnce(&checkerPoolTestView) -> R) -> R {
        let st = self.mu.lock().unwrap();
        let view = checkerPoolTestView {
            checkers: st.checkers.iter().map(|c| c.is_some()).collect(),
            held_by: st.held_by.clone(),
            file_associations: st.file_associations.clone(),
            request_associations: st.request_associations.clone(),
            has_cleanup_timer: st.cleanup_timer.is_some(),
            discarded: st.discarded,
            persistent: st.persistent_checker.is_some(),
            persistent_held: st.persistent_held,
        };
        f(&view)
    }
}

#[cfg(test)]
pub(crate) struct checkerPoolTestView {
    pub(crate) checkers: Vec<bool>,
    pub(crate) held_by: Vec<String>,
    pub(crate) file_associations: FxHashMap<P<SourceFile>, usize>,
    pub(crate) request_associations: FxHashMap<String, usize>,
    pub(crate) has_cleanup_timer: bool,
    pub(crate) discarded: bool,
    pub(crate) persistent: bool,
    pub(crate) persistent_held: bool,
}

impl checkerPool {
    // Go drops the pool's reference and lets the GC reclaim the checker. Region hook: the checker is parked with its
    // region until the pool is freed (see `parked`).
    fn dispose_checker(&self, checker: PooledChecker) {
        self.parked.lock().unwrap().push(checker);
    }
}

#[cfg(test)]
#[path = "checkerpool_test.rs"]
mod checkerpool_test;

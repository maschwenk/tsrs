//! Variances shared between the checkers of a program (`TSRS_SHARED_VARIANCE`, on by default, off under Go-compatible
//! history; docs/DEBUGGING.md, notes/perf-heavy-files-shared-variance.md).
//!
//! `getVariancesWorker` measures the variance of each type parameter of a generic interface, class or type alias by
//! relating two marker instantiations of it, structurally. For the large generics of UI and schema libraries that is
//! the most expensive single step of a check: MUI's `Components<Theme>` takes ~0.4 s and creates ~53k types, Zod's
//! `ZodOptional`, `ZodNullable`, ... and react-select's `ControlProps`, ... tens of milliseconds each. Every checker
//! measures again (variances are checker state, like everything else), so with 16 checkers the same declarations are
//! measured up to 16 times, and the files that need them first in each checker are the slowest of the run.
//!
//! This table keeps, per program, the variances that a checker measured from a clean start, and gives them to the
//! other checkers. TypeScript makes variances a function of the declaration: a measurement starts with
//! `resolutionStart` past the type resolutions in progress, so the cycles it can see are its own, and a circular
//! measurement restarts from the smallest symbol of the cycle, so the answer does not depend on where it was
//! entered. A checker that uses another's result is in the state it would be in had it measured the same declaration
//! earlier itself, which tsrs's default mode already has to (and is tested to) give the same output for
//! (notes/perf-order-independence.md: random checker assignments and visit orders print the same text).
//!
//! A result is published only from a measurement that
//! - started with an empty variance stack (no measurement of another generic around it, whose markers it could see);
//! - read no transient state: the flow memo's taint frame around it (flowmemo.rs) saw no in-progress or circular type
//!   resolution older than it and no `resolvingSignature`;
//! - added no diagnostic (an error found while resolving a member would otherwise be reported by one checker only).
//!
//! A checker asks the table only when its own variance stack is empty and it is not inside a measurement it claimed
//! from the table (a circular measurement restarts from the smallest symbol of the cycle with the stack emptied;
//! `shared_variance_depth` keeps that restart away from the table). If another checker is measuring the same
//! declaration it waits for that result instead of measuring it again. A checker that waits holds no claim, and a
//! checker that holds a claim never waits, so no checker waits for one that waits. If the measurement is not
//! published (or its checker unwinds), the waiters measure for themselves.
//!
//! The key is the program, the file index, the span and the kind of the declaration's first declaration node, and the
//! number of type parameters: the same in every checker of a program (programs are never freed, so a program's
//! address is never reused).
//!
//! `TSRS_SHARED_VARIANCE=shadow` measures locally even when the table has the answer and panics if they differ.
//! `TSRS_SHARED_VARIANCE=0` turns the table off, `=1` forces it on under Go-compatible history too.
//! `TSRS_SHARED_VARIANCE_STATS=1` prints the totals on stderr at exit.

use crate::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SharedVarianceMode {
    Off,
    On,
    Shadow,
}

fn env_mode() -> Option<SharedVarianceMode> {
    static MODE: OnceLock<Option<SharedVarianceMode>> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("TSRS_SHARED_VARIANCE").as_deref() {
        Ok("0" | "off") => Some(SharedVarianceMode::Off),
        Ok("1" | "on") => Some(SharedVarianceMode::On),
        Ok("shadow") => Some(SharedVarianceMode::Shadow),
        _ => None,
    })
}

/// The mode for a checker created now: the environment's; by default on, except under Go-compatible history.
pub(crate) fn shared_variance_mode() -> SharedVarianceMode {
    match env_mode() {
        Some(mode) => mode,
        None if tsrs_core::compat::go_compatible_history() => SharedVarianceMode::Off,
        None => SharedVarianceMode::On,
    }
}

fn stats_on() -> bool {
    static STATS: OnceLock<bool> = OnceLock::new();
    *STATS.get_or_init(|| std::env::var("TSRS_SHARED_VARIANCE_STATS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

static LOOKUPS: AtomicU64 = AtomicU64::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);
static WAITS: AtomicU64 = AtomicU64::new(0);
static PUBLISHED: AtomicU64 = AtomicU64::new(0);
static NOT_PUBLISHED: AtomicU64 = AtomicU64::new(0);
static SHADOW_CHECKS: AtomicU64 = AtomicU64::new(0);

fn count(counter: &AtomicU64) {
    if stats_on() || env_mode() == Some(SharedVarianceMode::Shadow) {
        // Relaxed: a statistic, read once at exit after the checker threads are joined.
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

fn total(counter: &AtomicU64) -> u64 {
    // Relaxed: read at exit, after the checker threads that counted were joined.
    counter.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SharedVarianceKey {
    program: usize,
    file: i32,
    pos: i32,
    end: i32,
    kind: Kind,
    params: u32,
}

enum Slot {
    /// A checker is measuring it.
    Measuring,
    Done(Box<[VarianceFlags]>),
}

struct Table {
    slots: Mutex<FxHashMap<SharedVarianceKey, Slot>>,
    published: Condvar,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| Table { slots: Mutex::new(FxHashMap::default()), published: Condvar::new() })
}

/// What a checker that asks the table does next.
pub(crate) enum SharedVariance {
    /// Use these variances.
    Found(Vec<VarianceFlags>),
    /// Measure, then `publish` (or drop the claim).
    Claimed(SharedVarianceClaim),
    /// Measure without publishing (no key, or shadow mode).
    Measure,
}

/// The right to publish a measurement; dropped unpublished (an impure measurement, or an unwinding checker), it
/// releases the declaration for the waiters to measure themselves.
pub(crate) struct SharedVarianceClaim {
    key: SharedVarianceKey,
    published: bool,
}

impl SharedVarianceClaim {
    pub(crate) fn publish(mut self, variances: &[VarianceFlags]) {
        let t = table();
        t.slots.lock().unwrap().insert(self.key, Slot::Done(variances.into()));
        self.published = true;
        count(&PUBLISHED);
        t.published.notify_all();
    }
}

impl Drop for SharedVarianceClaim {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let t = table();
        if let Ok(mut slots) = t.slots.lock() {
            slots.remove(&self.key);
        }
        count(&NOT_PUBLISHED);
        t.published.notify_all();
    }
}

impl Checker {
    fn shared_variance_key(&self, symbol: P<Symbol>, type_parameters: &[P<Type>]) -> Option<SharedVarianceKey> {
        let declaration = *symbol.declarations().first()?;
        let file = *self.file_index_map.get(&tsrs_ast::get_source_file_of_node(declaration)?)?;
        Some(SharedVarianceKey {
            program: std::ptr::from_ref(self.program).cast::<()>().addr(),
            file,
            pos: declaration.pos(),
            end: declaration.end(),
            kind: declaration.kind(),
            params: type_parameters.len() as u32,
        })
    }

    /// Asks the table for the variances of `symbol`, a generic this checker has not measured, with an empty variance
    /// stack. Waits while another checker measures it.
    pub(crate) fn shared_variance_lookup(&self, symbol: P<Symbol>, type_parameters: &[P<Type>]) -> SharedVariance {
        debug_assert!(self.variance_stack.is_empty() && self.shared_variance_depth == 0);
        let Some(key) = self.shared_variance_key(symbol, type_parameters) else {
            return SharedVariance::Measure;
        };
        count(&LOOKUPS);
        let t = table();
        let mut slots = t.slots.lock().unwrap();
        let mut waited = false;
        loop {
            match slots.get(&key) {
                Some(Slot::Done(variances)) => {
                    count(&HITS);
                    if self.shared_variance_mode == SharedVarianceMode::Shadow {
                        return SharedVariance::Measure;
                    }
                    return SharedVariance::Found(variances.to_vec());
                }
                Some(Slot::Measuring) => {
                    if !waited {
                        count(&WAITS);
                        waited = true;
                    }
                    slots = t.published.wait(slots).unwrap();
                }
                None => {
                    slots.insert(key, Slot::Measuring);
                    return SharedVariance::Claimed(SharedVarianceClaim { key, published: false });
                }
            }
        }
    }

    /// Shadow mode: the table's answer for `symbol`, if any, to compare with a local measurement.
    pub(crate) fn shared_variance_shadow_check(&self, symbol: P<Symbol>, type_parameters: &[P<Type>], measured: &[VarianceFlags]) {
        let Some(key) = self.shared_variance_key(symbol, type_parameters) else {
            return;
        };
        if let Some(Slot::Done(shared)) = table().slots.lock().unwrap().get(&key) {
            count(&SHADOW_CHECKS);
            if &**shared != measured {
                panic!("TSRS_SHARED_VARIANCE=shadow: variances of {} ({key:?}): shared {:?}, measured here {:?}", symbol.name(), shared, measured);
            }
        }
    }

    /// Prints the process totals when `TSRS_SHARED_VARIANCE_STATS` is set or in shadow mode (call at exit).
    pub fn shared_variance_finish() {
        if !stats_on() && env_mode() != Some(SharedVarianceMode::Shadow) {
            return;
        }
        eprintln!(
            "tsrs: shared variances: {} lookups, {} found ({} after waiting for another checker), {} published, {} measurements not published, {} shadow checks",
            total(&LOOKUPS),
            total(&HITS),
            total(&WAITS),
            total(&PUBLISHED),
            total(&NOT_PUBLISHED),
            total(&SHADOW_CHECKS)
        );
    }
}

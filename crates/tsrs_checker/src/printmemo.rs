//! A per-checker memo of top-level `type_to_string_ex` calls (`TSRS_TYPE_PRINT_MEMO`; on by default, off under
//! Go-compatible history). From can1357's microsoft/TypeScript perf/checker-speedups-v3 (15c911a9b, "memoized
//! outermost type printing per checker"), with tsrs's exactness rules (unioncache.rs, flowmemo.rs).
//!
//! Relation error elaboration prints the same few types many times, mostly in branches whose errors are then thrown
//! away (notes/perf-excalidraw-typefest.md: printing was 95% of such a site's cost). The first print of a type does
//! the lazy resolutions and instantiations the text needs; a later print with the same inputs (the type, the
//! enclosing declaration, the flags, and `variance_type_parameter`, which marker types print through) finds them
//! done and builds the same text.
//!
//! Exactness. A print is repeated from the memo only if a print with the same inputs, made after the first one,
//! - added no diagnostic, found no resolution cycle (which marks the resolutions in progress as circular and prints
//!   degraded text), deferred no node and no diagnostic (`print_events`);
//! - created no type, symbol or signature, so the memo never changes the number or the ids of the objects created;
//! - and did not see the instantiation count reset (a nested `checkExpression`).
//! Such a print reads only caches that only grow, so a later one takes the same path and builds the same text. A hit
//! adds the instantiations that print counted (`instantiation_count` for the TS2589 budget and the
//! `--extendedDiagnostics` total), so the counters are those of an unmemoized run. It is not used if it would reach
//! the 5,000,000-instantiation budget, or if it starts deeper in the instantiation stack than every recorded print
//! (a deeper start could reach the depth limit of 100 where the recorded prints did not).
//! Prints are not memoized while member resolution is in progress (`resolving_members`: a mapped or anonymous type
//! has empty members until its resolution completes and prints as `{}` meanwhile), inside another print
//! (`serialization_level`, whose nested prints can be cut short to "?"), or with a verbosity context.
//!
//! `TSRS_TYPE_PRINT_MEMO=shadow` prints again at every hit and panics if the text or the counts differ, or if that
//! print created anything or added an event. `=0` turns the memo off, `=1` forces it on under Go-compatible history
//! too. `TSRS_TYPE_PRINT_MEMO_STATS=1` prints the totals on stderr at exit.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::*;
use rustc_hash::FxHashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TypePrintMemoMode {
    Off,
    On,
    Shadow,
}

fn env_mode() -> Option<TypePrintMemoMode> {
    static MODE: OnceLock<Option<TypePrintMemoMode>> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("TSRS_TYPE_PRINT_MEMO").as_deref() {
        Ok("0" | "off") => Some(TypePrintMemoMode::Off),
        Ok("1" | "on") => Some(TypePrintMemoMode::On),
        Ok("shadow") => Some(TypePrintMemoMode::Shadow),
        _ => None,
    })
}

/// The mode for a checker created now: the environment's; by default on, except under Go-compatible history.
fn memo_mode() -> TypePrintMemoMode {
    match env_mode() {
        Some(mode) => mode,
        None if tsrs_core::compat::go_compatible_history() => TypePrintMemoMode::Off,
        None => TypePrintMemoMode::On,
    }
}

fn stats_on() -> bool {
    static STATS: OnceLock<bool> = OnceLock::new();
    *STATS.get_or_init(|| std::env::var("TSRS_TYPE_PRINT_MEMO_STATS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

static PRINTS: AtomicU64 = AtomicU64::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);
static STORES: AtomicU64 = AtomicU64::new(0);
static SHADOW_CHECKS: AtomicU64 = AtomicU64::new(0);

impl Checker {
    /// Prints the process totals when `TSRS_TYPE_PRINT_MEMO_STATS` is set or in shadow mode (call at exit).
    pub fn type_print_memo_finish() {
        if !stats_on() && env_mode() != Some(TypePrintMemoMode::Shadow) {
            return;
        }
        // Relaxed: statistics, read once at exit after the checker threads are joined.
        eprintln!(
            "tsrs: type print memo: {} memoizable prints, {} hits, {} stored, {} shadow checks",
            PRINTS.load(Ordering::Relaxed),
            HITS.load(Ordering::Relaxed),
            STORES.load(Ordering::Relaxed),
            SHADOW_CHECKS.load(Ordering::Relaxed)
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    t: P<Type>,
    enclosing_declaration: Option<P<Node>>,
    flags: TypeFormatFlags,
    variance_type_parameter: Option<P<Type>>,
}

struct Entry {
    /// Set once a print after the first one met the rules above.
    text: Option<String>,
    /// What that print added to `instantiation_count` and `total_instantiation_count`.
    instantiations: u32,
    total_instantiations: u32,
    /// The deepest instantiation stack a recorded print started at.
    depth: usize,
}

pub(crate) struct TypePrintMemo {
    mode: TypePrintMemoMode,
    entries: FxHashMap<Key, Entry>,
}

impl TypePrintMemo {
    pub(crate) fn new() -> Self {
        TypePrintMemo { mode: memo_mode(), entries: FxHashMap::default() }
    }

    #[inline]
    pub(crate) fn enabled(&self) -> bool {
        self.mode != TypePrintMemoMode::Off
    }
}

/// The counters a print must leave alone to be stored, and the instantiation counts it adds.
#[derive(Clone, Copy)]
struct Snapshot {
    events: u32,
    types: u32,
    symbols: u32,
    signatures: u32,
    instantiations: u32,
    total_instantiations: u32,
}

impl Checker {
    fn print_snapshot(&self) -> Snapshot {
        Snapshot {
            events: self.print_events,
            types: self.type_count,
            symbols: self.symbol_count,
            signatures: self.signature_count,
            instantiations: self.instantiation_count,
            total_instantiations: self.total_instantiation_count,
        }
    }

    /// `type_to_string_ex` at the top level without a verbosity context, through the memo.
    pub(crate) fn type_to_string_memoized(&mut self, t: P<Type>, enclosing_declaration: Option<P<Node>>, flags: TypeFormatFlags) -> String {
        if stats_on() {
            // Relaxed: a statistic, read once at exit.
            PRINTS.fetch_add(1, Ordering::Relaxed);
        }
        let key = Key { t, enclosing_declaration, flags, variance_type_parameter: self.variance_type_parameter };
        let depth = self.instantiation_stack.len();
        let mut hit = None;
        let seen_depth = match self.type_print_memo.entries.get(&key) {
            Some(entry) => {
                if let Some(text) = &entry.text {
                    if depth <= entry.depth && (self.instantiation_count as u64 + entry.instantiations as u64) < 5_000_000 {
                        hit = Some((text.clone(), entry.instantiations, entry.total_instantiations));
                    }
                }
                Some(entry.depth)
            }
            None => None,
        };
        if let Some((text, instantiations, total_instantiations)) = hit {
            if stats_on() {
                // Relaxed: a statistic, read once at exit.
                HITS.fetch_add(1, Ordering::Relaxed);
            }
            if self.type_print_memo.mode == TypePrintMemoMode::Shadow {
                self.shadow_check_print(t, enclosing_declaration, flags, &text, instantiations, total_instantiations);
            } else {
                self.instantiation_count += instantiations;
                self.total_instantiation_count = self.total_instantiation_count.wrapping_add(total_instantiations);
            }
            return text;
        }
        let before = self.print_snapshot();
        let result = self.type_to_string_worker(t, enclosing_declaration, flags, None);
        let after = self.print_snapshot();
        if after.events == before.events && after.instantiations >= before.instantiations {
            let created = after.types != before.types || after.symbols != before.symbols || after.signatures != before.signatures;
            // The first print only marks the inputs as seen: it does the lazy work later prints find done.
            let text = if seen_depth.is_some() && !created { Some(result.clone()) } else { None };
            if text.is_some() && stats_on() {
                // Relaxed: a statistic, read once at exit.
                STORES.fetch_add(1, Ordering::Relaxed);
            }
            let entry = Entry {
                text,
                instantiations: after.instantiations - before.instantiations,
                total_instantiations: after.total_instantiations.wrapping_sub(before.total_instantiations),
                depth: depth.max(seen_depth.unwrap_or(0)),
            };
            self.type_print_memo.entries.insert(key, entry);
        }
        result
    }

    #[cold]
    #[inline(never)]
    fn shadow_check_print(&mut self, t: P<Type>, enclosing_declaration: Option<P<Node>>, flags: TypeFormatFlags, text: &str, instantiations: u32, total_instantiations: u32) {
        // Relaxed: a statistic, read once at exit.
        SHADOW_CHECKS.fetch_add(1, Ordering::Relaxed);
        let before = self.print_snapshot();
        let result = self.type_to_string_worker(t, enclosing_declaration, flags, None);
        let after = self.print_snapshot();
        let mismatch = if result != text {
            Some(format!("text {result:?} vs memo {text:?}"))
        } else if after.events != before.events {
            Some("the print added an event".to_string())
        } else if after.types != before.types || after.symbols != before.symbols || after.signatures != before.signatures {
            Some("the print created a type, symbol or signature".to_string())
        } else if after.instantiations.wrapping_sub(before.instantiations) != instantiations
            || after.total_instantiations.wrapping_sub(before.total_instantiations) != total_instantiations
        {
            Some(format!(
                "instantiations {} / {} vs memo {} / {}",
                after.instantiations.wrapping_sub(before.instantiations),
                after.total_instantiations.wrapping_sub(before.total_instantiations),
                instantiations,
                total_instantiations
            ))
        } else {
            None
        };
        if let Some(why) = mismatch {
            panic!("TSRS_TYPE_PRINT_MEMO=shadow: a memoized print differs from printing again: {why}");
        }
    }
}

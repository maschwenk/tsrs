//! Checker-history compatibility (notes/perf-order-independence.md).
//!
//! A few checker caches in typescript-go let the printed output depend on what the checker happened to see first,
//! so with several checkers the text depends on which checker checked which file. tsrs keeps Go's behaviour by
//! default for parity with tsgo. This policy is independent of checker assignment and scheduling: the ordinary
//! locality assignment and work stealing remain unchanged. An explicitly named non-Go assignment uses canonical
//! behaviour, which makes those caches a function of the program alone. Every place that has the two behaviours
//! asks `go_compatible_history()`.

use std::sync::atomic::{AtomicU8, Ordering};

const UNSET: u8 = 0;
const CANONICAL: u8 = 1;
const DEFAULT_GO: u8 = 2;
const EXPLICIT_GO: u8 = 3;

static MODE: AtomicU8 = AtomicU8::new(UNSET);

/// Whether checker caches keep typescript-go's history-dependent behaviour.
///
/// Unless set explicitly, this is true when no assignment is named or `TSRS_CHECKER_ASSIGNMENT=go`.
/// `TSRS_HISTORY=canonical` opts into canonical history without changing checker scheduling.
pub fn go_compatible_history() -> bool {
    // A plain flag set before any checker exists; nothing is published through it.
    match MODE.load(Ordering::Relaxed) {
        UNSET => {
            let canonical = std::env::var("TSRS_HISTORY").is_ok_and(|v| v == "canonical");
            let go = !canonical && std::env::var("TSRS_CHECKER_ASSIGNMENT").map_or(true, |v| v.is_empty() || v == "go");
            // Racing first readers compute the same value from the environment.
            MODE.store(if go { DEFAULT_GO } else { CANONICAL }, Ordering::Relaxed);
            go
        }
        mode => mode == DEFAULT_GO || mode == EXPLICIT_GO,
    }
}

/// Whether Go history was explicitly selected by a harness or caller that previously expected static scheduling.
pub fn explicit_go_compatible_history() -> bool {
    // A plain flag set before any checker exists; nothing is published through it.
    MODE.load(Ordering::Relaxed) == EXPLICIT_GO
}

/// Sets the mode (the `--checkerAssignment` option). Call before creating checkers.
pub fn set_go_compatible_history(on: bool) {
    // See `go_compatible_history`.
    MODE.store(if on { EXPLICIT_GO } else { CANONICAL }, Ordering::Relaxed);
}

/// Selects the checker history required by tsgo reference baselines.
///
/// For harnesses whose baselines are tsgo's output: Go's behaviour unless `TSRS_CHECKER_ASSIGNMENT` names
/// another assignment or `TSRS_HISTORY=canonical` (then the canonical behaviour, to measure how the baselines
/// differ from it; the latter without naming an assignment, so the default scheduling, stealing included, is
/// used).
pub fn use_go_history_for_tsgo_baselines() {
    let other = std::env::var("TSRS_CHECKER_ASSIGNMENT").is_ok_and(|v| !v.is_empty() && v != "go")
        || std::env::var("TSRS_HISTORY").is_ok_and(|v| v == "canonical");
    set_go_compatible_history(!other);
}

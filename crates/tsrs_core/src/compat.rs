//! Go-compatible check history (notes/perf-order-independence.md).
//!
//! A few checker caches in typescript-go let the printed output depend on what the checker happened to see first,
//! so with several checkers the text depends on which checker checked which file. tsrs makes those caches a function
//! of the program alone by default ("canonical"). `--checkerAssignment go` / `TSRS_CHECKER_ASSIGNMENT=go` keep Go's
//! behaviour, bug for bug, for byte-identity with tsgo: the conformance, fourslash and tsc harnesses run that way,
//! because their baselines are tsgo's output with one checker in program order. Every place that has the two
//! behaviours asks `go_compatible_history()`.

use std::sync::atomic::{AtomicU8, Ordering};

const UNSET: u8 = 0;
const CANONICAL: u8 = 1;
const GO: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(UNSET);

/// Whether checker caches keep typescript-go's history-dependent behaviour. Unless set explicitly, true exactly when
/// `TSRS_CHECKER_ASSIGNMENT=go`.
pub fn go_compatible_history() -> bool {
    // A plain flag set before any checker exists; nothing is published through it.
    match MODE.load(Ordering::Relaxed) {
        UNSET => {
            let go = std::env::var("TSRS_CHECKER_ASSIGNMENT").is_ok_and(|v| v == "go");
            // Racing first readers compute the same value from the environment.
            MODE.store(if go { GO } else { CANONICAL }, Ordering::Relaxed);
            go
        }
        mode => mode == GO,
    }
}

/// Sets the mode (the `--checkerAssignment` option). Call before creating checkers.
pub fn set_go_compatible_history(on: bool) {
    // See `go_compatible_history`.
    MODE.store(if on { GO } else { CANONICAL }, Ordering::Relaxed);
}

/// For harnesses whose baselines are tsgo's output: Go's behaviour unless `TSRS_CHECKER_ASSIGNMENT` names another
/// assignment or `TSRS_HISTORY=canonical` (then the canonical behaviour, to measure how the baselines differ from it;
/// the latter without naming an assignment, so the default scheduling, stealing included, is used).
pub fn use_go_history_for_tsgo_baselines() {
    let other = std::env::var("TSRS_CHECKER_ASSIGNMENT").is_ok_and(|v| !v.is_empty() && v != "go")
        || std::env::var("TSRS_HISTORY").is_ok_and(|v| v == "canonical");
    set_go_compatible_history(!other);
}

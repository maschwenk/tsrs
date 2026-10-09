//! Research instrumentation (`spike/shared-graph-seed`): `TSRS_TIMELINE=1` records named instants of the run (front
//! end end, checker creation, the shared-graph seed, each checker's start, fork and end) relative to process start,
//! and prints them on stderr at exit as `timeline <ms> <thread> <name> <arg> <value>` lines.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static EVENTS: Mutex<Vec<(f64, &'static str, i64, f64)>> = Mutex::new(Vec::new());

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TSRS_TIMELINE").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Call first thing in `main`.
pub fn init() {
    START.get_or_init(Instant::now);
}

/// Records `name` now; `arg` is a checker index (or -1), `value` any number (thread CPU seconds, a count).
pub fn mark(name: &'static str, arg: i64, value: f64) {
    if !enabled() {
        return;
    }
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0;
    EVENTS.lock().unwrap().push((t, name, arg, value));
}

pub fn dump() {
    if !enabled() {
        return;
    }
    let mut events = EVENTS.lock().unwrap().clone();
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = String::new();
    for (t, name, arg, value) in events {
        out.push_str(&format!("timeline {t:9.3} {name} {arg} {value:.4}\n"));
    }
    eprint!("{out}");
}

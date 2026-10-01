//! tsrs-only: sub-phase timings and counts that `--extendedDiagnostics` prints after Go's table.
//!
//! Recording is always on (a mutex push per phase; phases are coarse). `record` adds to an existing row of the
//! same name, so a phase that runs in several pieces (per loader round, per diagnostics pass) accumulates.
//! Rows print in first-recorded order.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum PhaseValue {
    Time(Duration),
    Count(u64),
}

static PHASES: Mutex<Vec<(&'static str, PhaseValue)>> = Mutex::new(Vec::new());

pub fn record(name: &'static str, d: Duration) {
    let mut phases = PHASES.lock().unwrap();
    match phases.iter_mut().find(|(n, _)| *n == name) {
        Some((_, PhaseValue::Time(t))) => *t += d,
        Some(_) => {}
        None => phases.push((name, PhaseValue::Time(d))),
    }
}

pub fn count(name: &'static str, n: u64) {
    let mut phases = PHASES.lock().unwrap();
    match phases.iter_mut().find(|(n2, _)| *n2 == name) {
        Some((_, PhaseValue::Count(c))) => *c += n,
        Some(_) => {}
        None => phases.push((name, PhaseValue::Count(n))),
    }
}

pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = f();
    record(name, start.elapsed());
    result
}

pub fn snapshot() -> Vec<(&'static str, PhaseValue)> {
    PHASES.lock().unwrap().clone()
}

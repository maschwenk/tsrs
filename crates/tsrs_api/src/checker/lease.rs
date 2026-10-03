// Re-entrancy guard for the program's persistent API checker (Go `CheckerLifetimeAPI`).
//
// The API checker is a single exclusive slot per program (`checkerPool.persistentSem`). Pinned Go blocks
// on it unconditionally; if a request holding it is waiting for a client callback (sync MessagePack: on
// the same thread; async: another request) and the client issues a nested request that needs the same
// checker, nothing can make progress. Before taking the slot, the checker lane takes this per-program
// gate instead: uncontended requests proceed immediately, contended ones wait while the holder makes
// progress, and only a contended acquisition while some request on the connection is waiting on the
// client (`tsrs_api_transport::blocking_may_deadlock`) fails with the transport's bounded re-entrancy
// error. Outside a transport request (tests, in-process callers) it is a plain blocking lock.
//
// The gate map is keyed by the program's address. Entries are only created and used while the
// requesting snapshot keeps that program alive, and an entry is removed as soon as no lease or waiter
// refers to it, so a key never outlives its program while in use. Keys never leave this module.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rustc_hash::FxHashMap;
use tsrs_compiler::Program;

use super::host::{CheckerError, CheckerErrorKind, CheckerResult};

#[derive(Default)]
struct Gate {
    held: Mutex<bool>,
    released: Condvar,
}

static GATES: Mutex<Option<FxHashMap<usize, Arc<Gate>>>> = Mutex::new(None);

fn lock_gates() -> std::sync::MutexGuard<'static, Option<FxHashMap<usize, Arc<Gate>>>> {
    GATES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Exclusive right to acquire the program's API checker; released on drop.
pub(crate) struct ApiCheckerLease {
    key: usize,
    gate: Arc<Gate>,
}

const RESOURCE: &str = "the program's API checker";

pub(crate) fn acquire(program: &'static Program) -> CheckerResult<ApiCheckerLease> {
    let key = program as *const Program as usize;
    let gate = lock_gates().get_or_insert_with(FxHashMap::default).entry(key).or_default().clone();
    let lease = |gate: Arc<Gate>| ApiCheckerLease { key, gate };
    let result = (|| {
        let mut held = gate.held.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if !*held {
                *held = true;
                return Ok(());
            }
            if let Some(cx) = tsrs_api_transport::current_request() {
                if tsrs_api_transport::blocking_may_deadlock() {
                    return Err(CheckerError { kind: CheckerErrorKind::Internal, message: tsrs_api_transport::reentrancy::reentrancy_error(RESOURCE).message });
                }
                if cx.cancel.is_cancelled() {
                    return Err(CheckerError { kind: CheckerErrorKind::Internal, message: "ipc: connection closed".to_string() });
                }
                // Re-check shortly: a holder may start waiting on the client while we wait.
                held = gate.released.wait_timeout(held, Duration::from_millis(2)).unwrap_or_else(|e| e.into_inner()).0;
            } else {
                held = gate.released.wait(held).unwrap_or_else(|e| e.into_inner());
            }
        }
    })();
    match result {
        Ok(()) => Ok(lease(gate)),
        Err(e) => {
            forget_if_unused(key, &gate);
            Err(e)
        }
    }
}

fn forget_if_unused(key: usize, gate: &Arc<Gate>) {
    let mut gates = lock_gates();
    // One reference in the map plus the caller's: nobody else holds or waits on this gate.
    if Arc::strong_count(gate) == 2 && !*gate.held.lock().unwrap_or_else(|e| e.into_inner()) {
        if let Some(map) = gates.as_mut() {
            map.remove(&key);
        }
    }
}

impl Drop for ApiCheckerLease {
    fn drop(&mut self) {
        *self.gate.held.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.gate.released.notify_all();
        forget_if_unused(self.key, &self.gate);
    }
}

#[cfg(test)]
pub(crate) fn is_tracked(program: &'static Program) -> bool {
    lock_gates().as_ref().is_some_and(|m| m.contains_key(&(program as *const Program as usize)))
}

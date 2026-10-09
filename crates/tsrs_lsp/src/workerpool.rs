// The threads that run asynchronous request work (Go: `go func() { doAsyncWork() }()` in dispatchLoop, a fresh
// goroutine per request). docs/LSP.md "Threading model": not a thread per request, because every thread owns a
// leak arena (tsrs_core::ptr ARENA) that is never returned, so a thread per request would leak at least one arena
// chunk per request. Instead a process-wide pool of at most `max(4, available_parallelism)` threads, each with a
// 512 MB stack (the checker recurses as deeply as in the CLI; Go's goroutine stacks grow without a fixed limit).
// Threads are started on demand, up to the limit, and live for the rest of the process.
//
// Difference to Go: at most that many async jobs run at once; further jobs wait in FIFO order. A job that blocks
// (e.g. `sendClientRequest` waiting for the client) holds its thread while it waits.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, OnceLock};

pub(crate) const WORKER_STACK_SIZE: usize = 512 << 20;

type job = Box<dyn FnOnce() + Send>;

struct workerPool {
    state: Mutex<workerPoolState>,
    cond: Condvar,
    max_threads: usize,
}

struct workerPoolState {
    jobs: VecDeque<job>,
    threads: usize,
    idle: usize,
}

fn get_pool() -> &'static workerPool {
    static POOL: OnceLock<workerPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let parallelism = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        workerPool { state: Mutex::new(workerPoolState { jobs: VecDeque::new(), threads: 0, idle: 0 }), cond: Condvar::new(), max_threads: parallelism.max(4) }
    })
}

// Go `go f()` for request work.
pub(crate) fn go(f: impl FnOnce() + Send + 'static) {
    let pool = get_pool();
    let mut state = pool.state.lock().unwrap();
    state.jobs.push_back(Box::new(f));
    if state.idle == 0 && state.threads < pool.max_threads {
        state.threads += 1;
        let n = state.threads;
        drop(state);
        std::thread::Builder::new()
            .name(format!("lsp-worker-{}", n))
            .stack_size(WORKER_STACK_SIZE)
            .spawn(|| worker_loop(get_pool()))
            .expect("failed to spawn a language server worker thread");
    } else {
        drop(state);
        pool.cond.notify_one();
    }
}

fn worker_loop(pool: &'static workerPool) {
    let mut state = pool.state.lock().unwrap();
    loop {
        if let Some(job) = state.jobs.pop_front() {
            drop(state);
            run_or_exit(job);
            state = pool.state.lock().unwrap();
            continue;
        }
        state.idle += 1;
        state = pool.cond.wait(state).unwrap();
        state.idle -= 1;
    }
}

pub(crate) fn run_or_exit(f: impl FnOnce()) {
    f();
}

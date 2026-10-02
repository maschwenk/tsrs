use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) const defaultMinWaitTime: Duration = Duration::from_millis(50);
pub(crate) const defaultMaxWaitTime: Duration = Duration::from_millis(500);

// Go keeps these as package variables so tests can shorten them; nothing changes them here.
const minWaitTime: Duration = defaultMinWaitTime;
const maxWaitTime: Duration = defaultMaxWaitTime;

type debounceCallback = Arc<dyn Fn() + Send + Sync>;

// debounce batches filesystem events for one backend. Each *watcher
// owns one debounce instance, created lazily on first subscribe and
// living for the process lifetime. The background goroutine costs
// nothing when idle.
//
// Per-backend (rather than process-wide) isolation means a slow user
// callback on one backend cannot starve event delivery on the others.
//
// Internally uses a resettable latch: the loop blocks until trigger()
// is called, then coalesces for minWaitTime before firing callbacks.
//
// Go's latch channels (`waitCh` closed = signalled, `triggerCh` replaced on each trigger) are a mutex-guarded
// `notified` flag and trigger generation with a condition variable.
pub(crate) struct debounce {
    mu: Mutex<debounceState>,
    latch_mu: Mutex<latchState>,
    latch_cv: Condvar,
}

struct debounceState {
    callbacks: HashMap<usize, debounceCallback>,
    last_time: Option<Instant>,
}

#[derive(Default)]
struct latchState {
    notified: bool,
    trigger_gen: u64,
}

impl debounce {
    // debounce.go:42
    pub(crate) fn new() -> Arc<debounce> {
        let d = Arc::new(debounce {
            mu: Mutex::new(debounceState { callbacks: HashMap::new(), last_time: None }),
            latch_mu: Mutex::new(latchState::default()),
            latch_cv: Condvar::new(),
        });
        let looper = d.clone();
        std::thread::Builder::new().name("fswatch-debounce".to_string()).spawn(move || looper.loop_()).expect("failed to spawn fswatch debounce thread");
        d
    }

    // debounce.go:51
    // add registers a callback under key.
    pub(crate) fn add(&self, key: usize, cb: debounceCallback) {
        self.mu.lock().unwrap().callbacks.insert(key, cb);
    }

    // debounce.go:58
    // remove deregisters the callback for key.
    pub(crate) fn remove(&self, key: usize) {
        self.mu.lock().unwrap().callbacks.remove(&key);
    }

    // debounce.go:65
    // trigger wakes the debounce loop.
    pub(crate) fn trigger(&self) {
        let mut l = self.latch_mu.lock().unwrap();
        if !l.notified {
            l.notified = true;
        }
        l.trigger_gen += 1;
        self.latch_cv.notify_all();
    }

    // debounce.go:76
    fn loop_(&self) {
        loop {
            self.latch_wait();
            self.notify_if_ready();
        }
    }

    // debounce.go:83
    fn notify_if_ready(&self) {
        let mut st = self.mu.lock().unwrap();
        let now = Instant::now();
        // Go's zero lastTime makes the first gap larger than maxWaitTime.
        let gap_exceeded = st.last_time.is_none_or(|last| now.duration_since(last) > maxWaitTime);
        if gap_exceeded {
            st.last_time = Some(now);
            drop(st);
            self.fire_callbacks();
            return;
        }
        drop(st);
        self.coalesce_wait();
    }

    // debounce.go:97
    fn coalesce_wait(&self) {
        let l = self.latch_mu.lock().unwrap();
        let gen = l.trigger_gen;
        let (l, timeout) = self.latch_cv.wait_timeout_while(l, minWaitTime, |l| l.trigger_gen == gen).unwrap();
        drop(l);
        if timeout.timed_out() {
            self.fire_callbacks();
        }
        // Otherwise a new event triggered; fire on the next tick.
    }

    // debounce.go:110
    // fireCallbacks snapshots and invokes all registered callbacks.
    fn fire_callbacks(&self) {
        let mut st = self.mu.lock().unwrap();
        st.last_time = Some(Instant::now());
        let cbs: Vec<debounceCallback> = st.callbacks.values().cloned().collect();
        drop(st);

        self.latch_reset();

        for cb in cbs {
            cb();
        }
    }

    // debounce.go:140
    fn latch_wait(&self) {
        let l = self.latch_mu.lock().unwrap();
        let _l = self.latch_cv.wait_while(l, |l| !l.notified).unwrap();
    }

    // debounce.go:147
    fn latch_reset(&self) {
        let mut l = self.latch_mu.lock().unwrap();
        if l.notified {
            l.notified = false;
        }
    }
}

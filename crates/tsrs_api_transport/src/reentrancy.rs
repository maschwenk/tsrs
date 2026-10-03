// Re-entrancy policy for API handlers (see README "Re-entrancy").
//
// A client filesystem callback may issue API requests while the server request that triggered the
// callback is still running. In sync (MessagePack) mode the nested request runs on the very thread that
// is blocked in `Caller::call`; in async (JSON-RPC) mode it runs on another thread. Either way, if the
// nested request blocks on a non-reentrant resource (session mutex, exclusive checker lease) held by a
// request that is waiting for the client, nothing can make progress: the pinned Go server hangs there.
// tsrs instead reports a deliberate error. Handlers do not need the transport's types for this: the
// connection publishes the current request context in a thread-local for the duration of a request.
//
// When is a contended acquisition a deadlock?
// - Sync: only one top-level request runs at a time, so a request thread that finds the resource taken
//   while some request is blocked on the client is a nested request issued from a callback: the
//   conflict is certain and is reported immediately.
// - Async: requests run concurrently and the transport cannot tell whether the client's pending
//   callback is the one that issued this request (filesystem callbacks also come from worker threads
//   that carry no request identity). An unrelated slow callback plus ordinary contention is not a
//   deadlock (pinned Go just waits), so the waiter keeps waiting; it fails only after the contention
//   has coexisted with a blocked client call for the whole grace period (`ConnOptions::
//   reentrancy_grace`, default 10 s). A real async re-entry deadlock therefore ends with the error
//   after the grace period instead of hanging, and valid concurrency is never rejected early.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::handler::RequestContext;
use crate::message::ApiError;

pub const DEFAULT_ASYNC_GRACE: Duration = Duration::from_secs(10);

/// Connection-wide count of requests currently blocked waiting for a client callback answer.
#[derive(Debug)]
pub struct CallbackState {
    waiting: AtomicUsize,
    lock: Mutex<()>,
    changed: Condvar,
    sync: bool,
    grace: Duration,
}

impl Default for CallbackState {
    fn default() -> Self {
        CallbackState::new(false, DEFAULT_ASYNC_GRACE)
    }
}

impl CallbackState {
    pub(crate) fn new(sync: bool, grace: Duration) -> CallbackState {
        CallbackState { waiting: AtomicUsize::new(0), lock: Mutex::new(()), changed: Condvar::new(), sync, grace }
    }

    pub fn waiting_on_client(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }

    pub(crate) fn enter(self: &Arc<Self>) -> WaitingGuard {
        self.waiting.fetch_add(1, Ordering::SeqCst);
        self.notify();
        WaitingGuard(self.clone())
    }

    fn notify(&self) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.changed.notify_all();
    }

    fn wait_for_change(&self, timeout: Duration) {
        let g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.changed.wait_timeout(g, timeout);
    }
}

pub(crate) struct WaitingGuard(Arc<CallbackState>);

impl Drop for WaitingGuard {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::SeqCst);
        self.0.notify();
    }
}

thread_local! {
    static CURRENT: std::cell::RefCell<Option<RequestContext>> = const { std::cell::RefCell::new(None) };
}

/// Sets the current request context for the duration of `f` (restores the previous one after, so
/// nested sync requests see their own depth).
pub(crate) fn with_request<R>(cx: &RequestContext, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<RequestContext>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take();
            CURRENT.with(|c| *c.borrow_mut() = prev);
        }
    }
    let prev = CURRENT.with(|c| c.borrow_mut().replace(cx.clone()));
    let _restore = Restore(prev);
    f()
}

/// The request being handled on this thread, if any. `depth >= 1` means a sync nested request issued
/// from inside a client callback.
pub fn current_request() -> Option<RequestContext> {
    CURRENT.with(|c| c.borrow().clone())
}

thread_local! {
    /// (start, last check) of the current contended wait on this thread, for the async grace period.
    static EPISODE: Cell<Option<(Instant, Instant)>> = const { Cell::new(None) };
}

/// A gap between checks longer than this starts a new contention episode (wait loops re-check every
/// few milliseconds).
const EPISODE_GAP: Duration = Duration::from_millis(200);

/// Call repeatedly while waiting for a contended exclusive resource (e.g. a checker lease) that cannot
/// use `lock_for_request`; when it returns true, give up with `reentrancy_error`. True when waiting could
/// deadlock on a client callback: immediately on a sync connection while a request is blocked on the
/// client; on an async connection only once that has been the case for the whole grace period (see
/// module docs). Always false outside a request.
pub fn blocking_may_deadlock() -> bool {
    let Some(cx) = current_request() else { return false };
    if cx.callbacks.waiting_on_client() == 0 {
        EPISODE.with(|e| e.set(None));
        return false;
    }
    if cx.callbacks.sync {
        return true;
    }
    let now = Instant::now();
    let start = match EPISODE.with(|e| e.get()) {
        Some((start, last)) if now.duration_since(last) < EPISODE_GAP => start,
        _ => now,
    };
    EPISODE.with(|e| e.set(Some((start, now))));
    now.duration_since(start) >= cx.callbacks.grace
}

pub fn reentrancy_error(resource: &str) -> ApiError {
    ApiError::internal(format!(
        "api: client error: {resource} is in use by a request that is waiting on a client callback; \
         API requests made from inside a callback cannot use it until that callback returns"
    ))
}

/// Locks `mutex` for an API request without deadlocking on callback re-entry: waits while the holder
/// is making progress, and fails with `reentrancy_error` when the lock is contended while a request on
/// this connection is waiting for the client. Outside a request it is a plain blocking lock. Poisoned
/// locks are recovered (the connection already reported the panic).
pub fn lock_for_request<'a, T>(mutex: &'a Mutex<T>, resource: &str) -> Result<MutexGuard<'a, T>, ApiError> {
    let Some(cx) = current_request() else {
        return Ok(mutex.lock().unwrap_or_else(|e| e.into_inner()));
    };
    loop {
        match mutex.try_lock() {
            Ok(g) => {
                EPISODE.with(|e| e.set(None));
                return Ok(g);
            }
            Err(TryLockError::Poisoned(p)) => return Ok(p.into_inner()),
            Err(TryLockError::WouldBlock) => {}
        }
        if blocking_may_deadlock() {
            return Err(reentrancy_error(resource));
        }
        if cx.cancel.is_cancelled() {
            return Err(ApiError::internal("ipc: connection closed"));
        }
        // Woken early when a request starts waiting on the client; otherwise re-check shortly.
        cx.callbacks.wait_for_change(Duration::from_millis(2));
    }
}

// Re-entrancy policy for API handlers (see README "Re-entrancy").
//
// A client filesystem callback may issue API requests while the server request that triggered the
// callback is still running. In sync (MessagePack) mode the nested request runs on the very thread that
// is blocked in `Caller::call`; in async (JSON-RPC) mode it runs on another thread. Either way, if the
// nested request blocks on a non-reentrant resource (session mutex, exclusive checker lease) held by a
// request that is waiting for the client, nothing can make progress: the pinned Go server hangs there.
// tsrs instead reports a deliberate error. Handlers do not need the transport's types for this: the
// connection publishes the current request context in a thread-local for the duration of a request.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use crate::handler::RequestContext;
use crate::message::ApiError;

/// Connection-wide count of requests currently blocked waiting for a client callback answer.
#[derive(Debug, Default)]
pub struct CallbackState {
    waiting: AtomicUsize,
    lock: Mutex<()>,
    changed: Condvar,
}

impl CallbackState {
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

/// True when blocking now could deadlock: this thread serves a request and some request on the same
/// connection is waiting for the client. Check before blocking on an exclusive resource (e.g. a checker
/// lease) that cannot use `lock_for_request`.
pub fn blocking_may_deadlock() -> bool {
    current_request().is_some_and(|cx| cx.callbacks.waiting_on_client() > 0)
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
            Ok(g) => return Ok(g),
            Err(TryLockError::Poisoned(p)) => return Ok(p.into_inner()),
            Err(TryLockError::WouldBlock) => {}
        }
        if cx.callbacks.waiting_on_client() > 0 {
            return Err(reentrancy_error(resource));
        }
        if cx.cancel.is_cancelled() {
            return Err(ApiError::internal("ipc: connection closed"));
        }
        // Woken early when a request starts waiting on the client; otherwise re-check shortly.
        cx.callbacks.wait_for_change(Duration::from_millis(2));
    }
}

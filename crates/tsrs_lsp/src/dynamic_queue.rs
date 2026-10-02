use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use tsrs_core::context::{Context, ContextError};

// Inspired by Brian C. Mills' "Rethinking Classical Concurrency Patterns" talk:
// https://www.youtube.com/watch?v=5zXAHh5tJqQ
//
// This queue is a state machine, where each state is a channel, "idle" or "ready".
// Only one caller ever has the actual state struct at a time. The Get function
// will wait until the "ready" channel holds the state. Putting an item
// means grabbing the state from any channel, modifying it, and putting it
// back on the "ready" channel. Since this is all managed via contexts, any method
// can be cancelled while waiting for the state.
//
// (Rust: the state is the queue behind a mutex; "ready" is "the deque is non-empty", signaled through a condvar.
// Waiting for "ready" is cancelable through the context like Go's select on ctx.Done().)

// dynamic_queue.go:17
pub(crate) struct dynamicQueue<T> {
    inner: Arc<dynamicQueueInner<T>>,
}

struct dynamicQueueInner<T> {
    state: Mutex<dynamicQueueState<T>>,
    ready: Condvar,
}

// dynamic_queue.go:22
pub(crate) struct dynamicQueueState<T> {
    items: VecDeque<T>,
}

// dynamic_queue.go:26
pub(crate) fn new_dynamic_queue<T: Send + 'static>() -> dynamicQueue<T> {
    dynamicQueue { inner: Arc::new(dynamicQueueInner { state: Mutex::new(dynamicQueueState { items: VecDeque::new() }), ready: Condvar::new() }) }
}

impl<T: Send + 'static> dynamicQueue<T> {
    // dynamic_queue.go:35
    pub(crate) fn put(&self, ctx: &Context, item: T) -> Result<(), ContextError> {
        if let Some(err) = ctx.err() {
            return Err(err);
        }

        let mut state = self.get_any(ctx)?;

        state.items.push_back(item);
        drop(state);
        self.inner.ready.notify_one();
        Ok(())
    }

    // dynamic_queue.go:50
    pub(crate) fn get(&self, ctx: &Context) -> Result<T, ContextError> {
        if let Some(err) = ctx.err() {
            return Err(err);
        }

        let mut state = self.get_ready(ctx)?;

        let item = state.items.pop_front().unwrap();

        if state.items.is_empty() {
            state.items = VecDeque::new();
        } else {
            drop(state);
            self.inner.ready.notify_one();
        }
        Ok(item)
    }

    // dynamic_queue.go:76
    pub(crate) fn get_any(&self, ctx: &Context) -> Result<MutexGuard<'_, dynamicQueueState<T>>, ContextError> {
        // The state is only ever held for a push or a pop, so taking it never waits on another caller for
        // long; Go's select on ctx.Done() is the check before taking it (Put/Get) and the ready wait below.
        Ok(self.inner.state.lock().unwrap())
    }

    // dynamic_queue.go:87
    fn get_ready(&self, ctx: &Context) -> Result<MutexGuard<'_, dynamicQueueState<T>>, ContextError> {
        let inner = &self.inner;
        let notifier = Arc::clone(&self.inner);
        wait_until(ctx, &inner.ready, inner.state.lock().unwrap(), |s| !s.items.is_empty(), move || {
            drop(notifier.state.lock().unwrap());
            notifier.ready.notify_all();
        })
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner.state.lock().unwrap().items.len()
    }
}

// Waits on `cond` until `ready(&state)` holds or `ctx` is canceled (Go: a select on a channel and `ctx.Done()`).
// `wake` must lock the mutex and notify `cond`; it runs (on its own thread) when the context is canceled while
// waiting, so the waiter observes the cancellation without polling.
pub(crate) fn wait_until<'a, S>(
    ctx: &Context,
    cond: &Condvar,
    mut guard: MutexGuard<'a, S>,
    ready: impl Fn(&S) -> bool,
    wake: impl FnOnce() + Send + 'static,
) -> Result<MutexGuard<'a, S>, ContextError> {
    if ready(&guard) {
        return Ok(guard);
    }
    if let Some(err) = ctx.err() {
        return Err(err);
    }
    let stop = if ctx.done_is_nil() { None } else { Some(ctx.after_func(wake)) };
    let result = loop {
        if ready(&guard) {
            break Ok(guard);
        }
        if let Some(err) = ctx.err() {
            break Err(err);
        }
        // A deadline is observed when the context is polled; bound the wait by it.
        guard = match ctx.deadline() {
            Some(deadline) => {
                let now = std::time::Instant::now();
                let timeout = deadline.saturating_duration_since(now) + std::time::Duration::from_millis(1);
                cond.wait_timeout(guard, timeout).unwrap().0
            }
            None => cond.wait(guard).unwrap(),
        };
    };
    if let Some(stop) = stop {
        stop.stop();
    }
    result
}

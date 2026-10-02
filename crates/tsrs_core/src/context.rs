// Go `context.Context` as the compiler, project system and language server use it: `context.Background()`
// (`Context::default()`), values (`context.WithValue`; `core.WithRequestID`, `core.WithCheckerLifetime`,
// `locale.WithLocale` and other typed keys), and cancellation (`context.WithCancel`, `WithCancelCause`,
// `WithTimeout`, `context.AfterFunc`, `ctx.Err()`, `ctx.Done()`). A `Context` is an immutable chain of nodes
// shared through `Arc`, so cloning is a refcount increment, like copying Go's interface value.

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct Context {
    node: Option<Arc<ctxNode>>,
}

struct ctxNode {
    parent: Context,
    kind: ctxKind,
}

enum ctxKind {
    Value(TypeId, Arc<dyn Any + Send + Sync>),
    Cancel(Arc<cancelState>),
}

// Go `context.Canceled` / `context.DeadlineExceeded`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContextError {
    Canceled,
    DeadlineExceeded,
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::Canceled => f.write_str("context canceled"),
            ContextError::DeadlineExceeded => f.write_str("context deadline exceeded"),
        }
    }
}

type afterFunc = Box<dyn FnOnce() + Send>;

// Go's `cancelCtx` (plus `timerCtx`'s deadline). Children are canceled with their parent; `after` holds the
// `context.AfterFunc` callbacks, each run once on its own thread when the context is canceled (Go runs each in
// its own goroutine).
struct cancelState {
    deadline: Option<Instant>,
    inner: Mutex<cancelInner>,
    done: Condvar,
}

#[derive(Default)]
struct cancelInner {
    err: Option<ContextError>,
    cause: Option<String>,
    children: Vec<Weak<cancelState>>,
    after: Vec<(u64, afterFunc)>,
    next_after_id: u64,
}

fn spawn_after_func(f: afterFunc) {
    std::thread::Builder::new().name("context-afterfunc".to_string()).spawn(f).expect("failed to spawn context.AfterFunc thread");
}

impl cancelState {
    fn new(deadline: Option<Instant>) -> Arc<cancelState> {
        Arc::new(cancelState { deadline, inner: Mutex::new(cancelInner::default()), done: Condvar::new() })
    }

    fn err(&self) -> Option<ContextError> {
        let err = self.inner.lock().unwrap().err;
        if err.is_none() {
            if let Some(deadline) = self.deadline {
                if Instant::now() >= deadline {
                    self.cancel(ContextError::DeadlineExceeded, None);
                    return self.inner.lock().unwrap().err;
                }
            }
        }
        err
    }

    fn cancel(&self, err: ContextError, cause: Option<String>) {
        let (children, after) = {
            let mut inner = self.inner.lock().unwrap();
            if inner.err.is_some() {
                return;
            }
            inner.err = Some(err);
            inner.cause = Some(cause.unwrap_or_else(|| err.to_string()));
            (std::mem::take(&mut inner.children), std::mem::take(&mut inner.after))
        };
        self.done.notify_all();
        for child in children {
            if let Some(child) = child.upgrade() {
                child.cancel(err, None);
            }
        }
        for (_, f) in after {
            spawn_after_func(f);
        }
    }
}

// The cancellation state a new child must be registered with (Go `parentCancelCtx`).
fn nearest_cancel(ctx: &Context) -> Option<&Arc<cancelState>> {
    let mut cur = ctx.node.as_ref();
    while let Some(node) = cur {
        if let ctxKind::Cancel(state) = &node.kind {
            return Some(state);
        }
        cur = node.parent.node.as_ref();
    }
    None
}

// Go `context.CancelFunc`: cancels the context it was returned with. Calling it more than once is a no-op.
#[derive(Clone)]
pub struct CancelFunc(Arc<cancelState>);

impl CancelFunc {
    pub fn call(&self) {
        self.0.cancel(ContextError::Canceled, None);
    }
}

// Go `context.CancelCauseFunc`.
#[derive(Clone)]
pub struct CancelCauseFunc(Arc<cancelState>);

impl CancelCauseFunc {
    pub fn call(&self, cause: Option<String>) {
        self.0.cancel(ContextError::Canceled, cause);
    }
}

// Go's `stop func() bool` returned by `context.AfterFunc`.
pub struct AfterFuncStop {
    state: Option<Weak<cancelState>>,
    id: u64,
}

impl AfterFuncStop {
    // Reports whether the call stopped f from being run.
    pub fn stop(&self) -> bool {
        let Some(state) = self.state.as_ref().and_then(|s| s.upgrade()) else {
            return false;
        };
        let mut inner = state.inner.lock().unwrap();
        let before = inner.after.len();
        inner.after.retain(|(id, _)| *id != self.id);
        inner.after.len() != before
    }
}

impl Context {
    // Go `context.Background()`.
    pub fn background() -> Context {
        Context::default()
    }

    fn push(&self, kind: ctxKind) -> Context {
        Context { node: Some(Arc::new(ctxNode { parent: self.clone(), kind })) }
    }

    // Go `context.WithValue(ctx, key, value)` where the key is the value's type (callers use a newtype per key,
    // as Go code uses an unexported key type).
    pub fn with_value<T: Any + Send + Sync>(&self, value: T) -> Context {
        self.push(ctxKind::Value(TypeId::of::<T>(), Arc::new(value)))
    }

    // Go `ctx.Value(key).(T)`.
    pub fn value<T: Any + Send + Sync>(&self) -> Option<&T> {
        let mut cur = self.node.as_ref();
        while let Some(node) = cur {
            if let ctxKind::Value(id, value) = &node.kind {
                if *id == TypeId::of::<T>() {
                    return value.downcast_ref::<T>();
                }
            }
            cur = node.parent.node.as_ref();
        }
        None
    }

    fn with_cancel_state(&self, deadline: Option<Instant>) -> (Context, Arc<cancelState>) {
        let parent = nearest_cancel(self);
        let deadline = match (deadline, parent.and_then(|p| p.deadline)) {
            (Some(d), Some(pd)) => Some(d.min(pd)),
            (d, pd) => d.or(pd),
        };
        let state = cancelState::new(deadline);
        if let Some(parent) = parent {
            match parent.err() {
                Some(err) => state.cancel(err, None),
                None => {
                    let mut inner = parent.inner.lock().unwrap();
                    match inner.err {
                        Some(err) => {
                            drop(inner);
                            state.cancel(err, None);
                        }
                        None => {
                            inner.children.retain(|c| c.strong_count() > 0);
                            inner.children.push(Arc::downgrade(&state));
                        }
                    }
                }
            }
        }
        (self.push(ctxKind::Cancel(state.clone())), state)
    }

    // Go `context.WithCancel(parent)`.
    pub fn with_cancel(&self) -> (Context, CancelFunc) {
        let (ctx, state) = self.with_cancel_state(None);
        (ctx, CancelFunc(state))
    }

    // Go `context.WithCancelCause(parent)`.
    pub fn with_cancel_cause(&self) -> (Context, CancelCauseFunc) {
        let (ctx, state) = self.with_cancel_state(None);
        (ctx, CancelCauseFunc(state))
    }

    // Go `context.WithTimeout(parent, timeout)`. The deadline is observed when the context is polled (`err`,
    // `is_canceled`, `wait`, `wait_timeout`); expiry alone does not run `after_func` callbacks.
    pub fn with_timeout(&self, timeout: Duration) -> (Context, CancelFunc) {
        let (ctx, state) = self.with_cancel_state(Some(Instant::now() + timeout));
        (ctx, CancelFunc(state))
    }

    // Go `ctx.Err()`: None while the context is live.
    pub fn err(&self) -> Option<ContextError> {
        nearest_cancel(self).and_then(|s| s.err())
    }

    pub fn is_canceled(&self) -> bool {
        self.err().is_some()
    }

    // Go `context.Cause(ctx)`.
    pub fn cause(&self) -> Option<String> {
        let state = nearest_cancel(self)?;
        state.err()?;
        state.inner.lock().unwrap().cause.clone()
    }

    // Go `ctx.Done() == nil`: the context can never be canceled (no WithCancel/WithTimeout in its chain).
    pub fn done_is_nil(&self) -> bool {
        nearest_cancel(self).is_none()
    }

    // Go `ctx.Deadline()`.
    pub fn deadline(&self) -> Option<Instant> {
        nearest_cancel(self).and_then(|s| s.deadline)
    }

    // Go `<-ctx.Done()`: blocks until the context is canceled (forever when `done_is_nil()`).
    pub fn wait(&self) {
        while !self.wait_timeout(Duration::from_secs(3600)) {}
    }

    // Go `select { case <-ctx.Done(): ...; case <-time.After(timeout): ... }`: true if the context is canceled
    // within `timeout`.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let Some(state) = nearest_cancel(self) else {
            std::thread::sleep(timeout);
            return false;
        };
        let end = Instant::now() + timeout;
        let mut inner = state.inner.lock().unwrap();
        loop {
            if inner.err.is_some() {
                return true;
            }
            let now = Instant::now();
            let mut until = end;
            if let Some(deadline) = state.deadline {
                if now >= deadline {
                    drop(inner);
                    state.cancel(ContextError::DeadlineExceeded, None);
                    return true;
                }
                until = until.min(deadline);
            }
            if now >= end {
                return false;
            }
            inner = state.done.wait_timeout(inner, until - now).unwrap().0;
        }
    }

    // Go `context.AfterFunc(ctx, f)`: runs `f` on its own thread once the context is canceled (right away if it
    // already is). Never runs when `done_is_nil()`.
    pub fn after_func(&self, f: impl FnOnce() + Send + 'static) -> AfterFuncStop {
        let Some(state) = nearest_cancel(self) else {
            return AfterFuncStop { state: None, id: 0 };
        };
        if state.err().is_some() {
            spawn_after_func(Box::new(f));
            return AfterFuncStop { state: None, id: 0 };
        }
        let mut inner = state.inner.lock().unwrap();
        if inner.err.is_some() {
            drop(inner);
            spawn_after_func(Box::new(f));
            return AfterFuncStop { state: None, id: 0 };
        }
        let id = inner.next_after_id;
        inner.next_after_id += 1;
        inner.after.push((id, Box::new(f)));
        AfterFuncStop { state: Some(Arc::downgrade(state)), id }
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("request_id", &get_request_id(self))
            .field("checker_lifetime", &get_checker_lifetime(self))
            .field("err", &self.err())
            .finish()
    }
}

// core/context.go

struct requestIDKey(String);

// context.go:14
pub fn with_request_id(ctx: &Context, id: &str) -> Context {
    ctx.with_value(requestIDKey(id.to_string()))
}

// context.go:18
pub fn get_request_id(ctx: &Context) -> &str {
    match ctx.value::<requestIDKey>() {
        Some(id) => &id.0,
        None => "",
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum CheckerLifetime {
    #[default]
    Temporary = 0,
    Diagnostics = 1,
    API = 2,
}

struct checkerLifetimeKey(CheckerLifetime);

// context.go:33
pub fn with_checker_lifetime(ctx: &Context, lifetime: CheckerLifetime) -> Context {
    ctx.with_value(checkerLifetimeKey(lifetime))
}

// context.go:37
pub fn get_checker_lifetime(ctx: &Context) -> CheckerLifetime {
    match ctx.value::<checkerLifetimeKey>() {
        Some(lifetime) => lifetime.0,
        None => CheckerLifetime::Temporary,
    }
}

// locale/locale.go. Messages are English only; a locale keeps its tag text so the API ports.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Locale(pub String);

impl Locale {
    // locale.go `Default`.
    pub const DEFAULT: Locale = Locale(String::new());

    pub fn string(&self) -> String {
        self.0.clone()
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

struct localeKey(Locale);

// locale.go `WithLocale`.
pub fn with_locale(ctx: &Context, locale: Locale) -> Context {
    ctx.with_value(localeKey(locale))
}

// locale.go `FromContext`.
pub fn locale_from_context(ctx: &Context) -> Locale {
    match ctx.value::<localeKey>() {
        Some(locale) => locale.0.clone(),
        None => Locale::DEFAULT,
    }
}

// locale.go `HasLocale`.
pub fn has_locale(ctx: &Context) -> bool {
    ctx.value::<localeKey>().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_propagates_to_children_and_runs_after_funcs() {
        let bg = Context::background();
        assert!(bg.done_is_nil());
        let ctx = with_request_id(&with_checker_lifetime(&bg, CheckerLifetime::Diagnostics), "42");
        let (parent, cancel) = ctx.with_cancel();
        let (child, _child_cancel) = with_request_id(&parent, "43").with_cancel();
        assert_eq!(get_request_id(&child), "43");
        assert_eq!(get_request_id(&parent), "42");
        assert_eq!(get_checker_lifetime(&child), CheckerLifetime::Diagnostics);
        assert!(!child.done_is_nil());
        let (tx, rx) = std::sync::mpsc::channel();
        child.after_func(move || tx.send(()).unwrap());
        assert!(child.err().is_none());
        cancel.call();
        assert_eq!(parent.err(), Some(ContextError::Canceled));
        assert_eq!(child.err(), Some(ContextError::Canceled));
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (late, _) = parent.with_cancel();
        assert!(late.is_canceled());
        assert!(!with_request_id(&bg, "x").is_canceled());
    }

    #[test]
    fn timeout_expires() {
        let (ctx, _cancel) = Context::background().with_timeout(Duration::from_millis(10));
        assert!(ctx.wait_timeout(Duration::from_secs(5)));
        assert_eq!(ctx.err(), Some(ContextError::DeadlineExceeded));
    }
}

# lsp-api: compiler- and checker-side API for the language service

Wave `api` of the language-server port: `tsrs_core::context::Context`, the pluggable checker pool and program reuse in
`tsrs_compiler`, and the checker's language-service API. The code is the reference now (docs/LSP.md has the
threading model and the checker pool); this note keeps the Go -> Rust mapping of `context.Context` and the deviations
that still hold.

## `tsrs_core::context::Context` (Go `context.Context`)

`crates/tsrs_core/src/context.rs`.

| Go | Rust |
| --- | --- |
| `context.Background()` | `Context::background()` / `Context::default()` |
| `context.WithValue(ctx, k, v)` / `ctx.Value(k).(T)` | `ctx.with_value(v)` / `ctx.value::<T>()` (the key is the value's type: use a private newtype per key, as Go uses an unexported key type) |
| `core.WithRequestID` / `core.GetRequestID` | `with_request_id(&ctx, id)` / `get_request_id(&ctx) -> &str` |
| `core.WithCheckerLifetime` / `core.GetCheckerLifetime` | `with_checker_lifetime` / `get_checker_lifetime` (`CheckerLifetime` moved here unchanged) |
| `locale.WithLocale` / `locale.FromContext` / `locale.HasLocale` | `with_locale` / `locale_from_context` / `has_locale` (`Locale(String)`, English only) |
| `context.WithCancel` / `CancelFunc` | `ctx.with_cancel() -> (Context, CancelFunc)`; `cancel.call()`; parent cancellation propagates to children (weak child list), a child of a canceled parent starts canceled |
| `context.WithCancelCause` / `context.Cause` | `with_cancel_cause()` / `CancelCauseFunc::call(Some(cause))` / `ctx.cause()` |
| `context.WithTimeout` | `with_timeout(d)` (see Deviations) |
| `ctx.Err()`, `context.Canceled`, `context.DeadlineExceeded` | `ctx.err() -> Option<ContextError>`, `ContextError::{Canceled, DeadlineExceeded}`; `ctx.is_canceled()` |
| `ctx.Done() == nil` | `ctx.done_is_nil()` (no cancel node in the chain) |
| `<-ctx.Done()` / `select` with timeout | `ctx.wait()` / `ctx.wait_timeout(d) -> bool` |
| `context.AfterFunc(ctx, f)` / `stop()` | `ctx.after_func(f) -> AfterFuncStop` / `stop.stop()`; `f` runs on its own thread (Go: own goroutine) |

Signatures take `ctx: &Context` (Go passes the interface by value; `&Context` avoids a refcount per call).

## Deviations that still hold

- `Context::with_timeout`: the deadline is observed when the context is polled (`err`, `is_canceled`, `wait*`);
  expiry alone does not fire `after_func` callbacks (Go fires them from its timer). Callers in Go (session watch
  requests) only poll / wait.
- `CheckerHandle` (crates/tsrs_compiler/src/checkerpool.rs) can hold a `MutexGuard`, so it is `!Send`: a handle must
  be released on the thread that acquired it. Go's release funcs may run anywhere; no LS/project call site moves a
  checker between goroutines mid-use, but a pool release triggered from another thread (e.g. `context.AfterFunc`)
  must not own a handle.
- The built-in pool's `getCheckerNonExclusive` / `getCheckerForFileNonExclusive` lock the checker (Go returns it
  unlocked): Rust hands out `&mut Checker` only under the lock (docs/LSP.md, "Nested checker acquisition").
- `new_program` runs the pool factory after `verify_compiler_options` (Go: before), because the factory takes
  `&'static Program`. Unobservable: pools create checkers lazily.
- External-pool fallback in `collectCheckerDiagnosticsFromFiles` / `GetDeclarationDiagnostics`: Go's work group ->
  the compiler's worker pool; single-threaded runs go last-queued first like Go's work group.

Status (2026-10-10): the wave-era limitations this note listed are gone: `is_canceled()` polls the request context
(docs/LSP.md, Cancellation), project references and emit are ported, and programs and checkers are freed with
regions (docs/LSP.md, memory plan).

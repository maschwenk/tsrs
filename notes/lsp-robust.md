# lsp-robust: cancellation, builtin file watcher, watched-file events (phase 4)

Wave agent `robust`, branch `lsp-robust`.

## 1. Cancellation

Audit of every Go cancellation point against the port:

| Go | Rust | state |
| --- | --- | --- |
| checker `c.ctx` set/cleared in `checkSourceFile` (checker.go:2237), `isCanceled` (utilities.go:1712), `checkNotCanceled` (utilities.go:1716), `wasCanceled` | `checker_02.rs` `check_source_file`, `utilities.rs` | present since the api wave except `c.ctx = nil` at the end of `checkSourceFile`, now added (a later request on the same checker saw the previous request's context) |
| polling: `checkSourceElements`, `checkDeferredNodes`, `checkContextualDeprecations`, the two unused checks in `checkSourceFile`, `getDiagnostics` / `GetGlobalDiagnostics` (`checkNotCanceled`), `createRecoveryBoundary` (nodecopy.go:196) | same functions | all present |
| `WasCanceled` (exports.go:171) | `exports.rs` | present |
| project pool: dispose a canceled checker on release (`createRelease`, `getPersistentChecker`) | `tsrs_project/checkerpool.rs` | present (unchanged) |
| compiler `getDiagnosticsOfFile` `WasCanceled` (program.go:1513) | `program.rs` | present |
| ls `ctx.Err()`: crossproject (4), importTracker, codelens, findallreferences (2), signaturehelp (2), folding (2), symbols (2), inlay_hints, semantictokens (2) | `tsrs_ls` | all present; `autoimport/registry.go` (8) belongs to the placeholder registry (actions wave) |
| server: handler registrations' `ctx.Err()` (notification, request, language-service, auto-import, multi-project), `handleCodeLensResolve`, dispatch `handleError` -> `RequestCancelled`, `$/cancelRequest` -> cancel the request context | `tsrs_lsp/server.rs` | all present |
| session debounce waits (`ctx.Done()`), background queue | `tsrs_project` | present |

Cost: `cancelState` got an `AtomicBool` set when the context is canceled, so `Context::err()` on a live cancelable
context is one atomic load (plus the deadline check for timeout contexts) instead of a mutex lock. The CLI passes
`Context::background()`, whose `err()` returns at the first `None` node.

Tests: checkerpool_test.go `TestCheckerPoolCanceledCheckerDisposal`, `TestCheckerPoolRequestAssociationCleanupOnDisposal`,
`TestCheckerPoolAPICheckerDisposedOnCancel` ported (`checkerpool_test.rs`, 27 pool tests pass). Go has no server-level
cancellation test (server_test.go / dynamic_queue_test.go only cancel the server context; those are already ported).

Gates: conformance 13,458 pass / 2 codes / 2 fail; `TSRS_LAZY_MEMBERS=0 --baselines types,symbols` 12,779 / 12,779;
fourslash 3,325 pass; webpack CLI (hyperfine, 10 runs) 374.9 ± 16.6 ms before vs 377.1 ± 17.1 ms after (noise).

## Shared-file edits

- `crates/tsrs_core/src/context.rs`: `cancelState.canceled` atomic fast path in `err()`.
- `crates/tsrs_checker/src/checker_02.rs`: `self.ctx = None` at the end of `check_source_file` (checker.go:2270).

## Deviations

## Needs from others

## Doubts

- The ported pool tests compare checker addresses after disposal like Go compares pointers. Disposed checkers are
  leaked today; once the mem wave frees them, a fresh checker may reuse the address and these asserts could flake.

# lsp-robust: cancellation, builtin file watcher, watched-file events, state baselines (phase 4)

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

## 2. Builtin file watcher

| Go (`ts-ref/tsc/internal/…`) | Rust |
| --- | --- |
| `fswatch/watcher.go` | `crates/tsrs_fswatch/src/watcher.rs` (+ Go's sentinel errors / `%w` chains as `Error { message, wraps, errno }`) |
| `fswatch/event.go`, `debounce.go`, `pathcompare.go`, `pathkey.go` | `event.rs`, `debounce.rs`, `pathcompare.rs`, `pathkey.rs` |
| `fswatch/canonicalize_darwin.go`, `canonicalize_other.go` | `canonicalize.rs` (`cfg(target_os = "macos")` like the build tags) |
| `fswatch/fsevents_darwin.go` | `fsevents_darwin.rs` |
| `fswatch/fsevents_darwin_ffi.go` (+ the `.s` trampolines) | `fsevents_darwin_ffi.rs`: `extern "C"` CoreFoundation / CoreServices (`#[link(kind = "framework")]`) / libdispatch; all FSEvents `unsafe` is here |
| `fswatch/kqueue.go` | `kqueue.rs` (macOS / BSD, `libc`) |
| `fswatch/inotify_linux.go` | `inotify_linux.rs` (checked with `cargo check --target x86_64-unknown-linux-gnu`; not run) |
| `fswatch/walkdir_unix.go` | `walkdir_unix.rs` |
| `lsp/lspwatcher/lspwatcher.go` | `crates/tsrs_lsp/src/lspwatcher.rs` |
| server.go builtin-watcher branches (`WatchFiles`, `UnwatchFiles`, `handleInitialized`, `handleShutdown`) | `server.rs`: `builtin_watcher: OnceLock<Arc<lspwatcher::Watcher>>`, created exactly where Go creates it (no dynamic watch registration and `fswatch::default().has_fast_recursive_backend()`) |

Tests: eventlist_test.go (7), watcher_test.go (32 of the macOS-runnable tests: path comparer / rebase / symlink /
case-sensitivity unit tests and the per-backend CRUD, coalescing, multiple-subscription, lifecycle, non-recursive,
WatchFile, atomic-save tests, run against FSEvents and kqueue), with Go's retry wrapper (testutil_test.go: 3
attempts, scaled timeouts); lspwatcher_test.go (all 15). New dependency: `libc` (0.2, already in the registry
cache) for kqueue / inotify / pipe / pathconf / stat.

Not ported: `fanotify_linux.go` (Linux: `fanotify()` is unavailable, so `default()` is inotify, Go's own fallback on
kernels without fanotify), `windows.go` (Windows builds get an unavailable watcher), `walkDirGeneric` (only used where
the unix walker does not exist), the Go tests that need those, `TestSubscribeNoGoroutineLeak` (goroutine counting),
the NFD / shared-stream FSEvents test files (fsevents_darwin_nfd_test.go, fsevents_darwin_shared_test.go,
fsevents_darwin_ffi_arm64_test.go) and about 50 further per-backend scenarios of watcher_test.go (symlink roots,
consolidation, denied subdirectories, rename-dir trees, replace dir/file, round-trip renames).

## 3. Watched-file events end to end

`Session.DidChangeWatchedFiles` was already ported (project wave); both the client notification and the builtin
watcher feed it. `tools/oracle/lsp/lsp_oracle.py` gained `--closed`, `--disk-edits`, `--disk-edit-at`,
`--watch-client`, `--watch-wait` (see the docstring) and fixtures `tools/oracle/lsp/fixtures/{multi,refs}`. Results
(docs/LSP.md "Watched files"): `multi` 128/128 and `refs` (two tsconfigs, app references lib) 65/65 responses equal
with `tsgo-ref`, both with client watching and with the builtin watcher; in all four sessions the disk edit changes
the open file's diagnostics (TS2459 / TS2305) in both servers and the revert clears them.

## 4. Fourslash state baselines

`fourslash/statebaseline.go` (whole) -> `crates/tsrs_fourslash/src/statebaseline.rs`; `testutil/fsbaselineutil/differ.go`
(`FSDiffer`, `BaselineFSwithDiff`, `addFsEntryDiff`, `SanitizeInternalSymbolName`) -> `fsbaselineutil.rs`. 17 of the
20 state-baseline tests pass; the other 3 stop at `@tsc` command lines (2; `tsc -b` needs emit, not ported) and
`workspace/willRenameFiles` (1; actions wave). `@tsc` (8 tests in all) stays "not ported".

## Gates (2026-10-02)

- Conformance 13,458 pass / 2 codes / 2 fail; `TSRS_LAZY_MEMBERS=0 --baselines types,symbols` 12,779 / 12,779 (after
  item 1 and at the end).
- Fourslash 3,325 -> 3,342 pass (pass list a superset of the starting one).
- webpack CLI (hyperfine, 10 runs): base 374.9 ± 16.6 ms vs item 1 377.1 ± 17.1 ms; final 366.8 ± 2.1 ms vs base
  1.00 ± 0.01 (noise).
- `cargo check --workspace --tests`: 0 warnings. `cargo test`: tsrs_fswatch 39, tsrs_lsp 41 (26 + 15 lspwatcher),
  tsrs_project 108 (+3), tsrs_fourslash lib 2.

## Shared-file edits

- `crates/tsrs_core/src/context.rs`: `cancelState.canceled` atomic fast path in `err()`.
- `crates/tsrs_checker/src/checker_02.rs`: `self.ctx = None` at the end of `check_source_file` (checker.go:2270).
- `crates/tsrs_project/src/checkerpool_test.rs`: 3 cancellation tests (no change to `checkerpool.rs`).
- `crates/tsrs_lsp/src/server.rs`, `lib.rs`, `Cargo.toml`: builtin watcher wiring, `pub mod lspwatcher`, depends on
  `tsrs_fswatch` (path dependency; not in `[workspace.dependencies]` to keep the root manifest untouched).
- `crates/tsrs_fourslash/src/fourslash.rs` (state baseline creation, shared `Arc` map FS), `lib.rs`, `Cargo.toml`
  (+ `tsrs_compiler`).
- `Cargo.lock`: `libc`, `tsrs_fswatch`.

## Deviations

- FSEvents callback: Go cannot run Go code on the GCD thread without cgo, so an assembly callback copies the payload
  into a pipe read by a goroutine. The Rust `extern "C"` callback classifies the events on the stream's serial GCD
  queue itself (same `fs_events_callback`); teardown keeps Go's order (stop, invalidate, wait for the queue,
  release), so no callback outlives its stream.
- kqueue: entry maps live in the backend state keyed by a map id (Go shares `*dirEntry` pointers between maps and
  `fdToEntry`), and `compareDir` holds the backend mutex for its whole run (Go drops it around directory reads).
  Only the event-loop thread mutates entry maps after `subscribe`, so this changes blocking, not results. Go's
  `removeSubsForEntriesLocked` compares `&sub.entries` field addresses (never equal across subscriptions); kept as
  identity of the given subscription.
- inotify: Go guards `subscriptions` with watcherBase.mu; here it has its own mutex, always taken after watcherBase.mu
  where Go holds that lock.
- `walkDir` uses `std::fs::read_dir` (readdir order, d_type with lstat fallback) instead of raw getdents.
- `lspwatcher` pending events keep insertion order (Go: map order) when flushed.
- The fourslash client lacks dynamic watch registration, so on macOS fourslash servers now run the builtin watcher
  over real-disk paths that mostly do not exist (registrations fail and are logged), exactly like Go's fourslash on
  macOS.

## Needs from others

- actions wave: `workspace/willRenameFiles` (1 state-baseline test).
- mem wave: the ported pool tests compare checker addresses after disposal like Go compares pointers; once disposed
  checkers are freed, a new checker may reuse the address and those asserts could flake.

## Doubts

- The FSEvents test runs under a machine load of 20-60 needed Go's retry once in a few runs (timeouts), never with
  `--test-threads=2`; Go's suite has the same retry for the same reason.
- The oracle's disk edits modify fixture files in place and revert them; an interrupted run leaves a fixture edited.

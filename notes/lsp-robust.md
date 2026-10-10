# lsp-robust: cancellation, builtin file watcher, watched-file events, state baselines (phase 4)

Wave `robust` of the language-server port. Results are in docs/LSP.md (Progress, "Watched files", Cancellation). This
note keeps the `fswatch` port's deviations from Go and what was not ported: the rationale for the `unsafe` FFI code in
`crates/tsrs_fswatch`.

Cancellation: every Go cancellation point (checker polling, `WasCanceled`, pool disposal of canceled checkers, the
`ctx.Err()` checks in `ls` and the server's handler registrations, `$/cancelRequest`) was audited against the port;
the one gap was `c.ctx = nil` at the end of `checkSourceFile` (a later request on the same checker saw the previous
request's context), now set. `Context::err()` on a live cancelable context is one atomic load (an `AtomicBool` in
`cancelState`) instead of a mutex lock. webpack CLI (hyperfine, 10 runs): 366.8 ± 2.1 ms vs base 374.9 ± 16.6 ms,
1.00 ± 0.01 (noise).

## Builtin file watcher: what is where

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

Not ported: `fanotify_linux.go` (Linux: `fanotify()` is unavailable, so `default()` is inotify, Go's own fallback on
kernels without fanotify), `windows.go` (Windows builds get an unavailable watcher), `walkDirGeneric` (only used where
the unix walker does not exist), the Go tests that need those, `TestSubscribeNoGoroutineLeak` (goroutine counting),
the NFD / shared-stream FSEvents test files (fsevents_darwin_nfd_test.go, fsevents_darwin_shared_test.go,
fsevents_darwin_ffi_arm64_test.go) and about 50 further per-backend scenarios of watcher_test.go (symlink roots,
consolidation, denied subdirectories, rename-dir trees, replace dir/file, round-trip renames).

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

## Doubts

- The FSEvents test runs under a machine load of 20-60 needed Go's retry once in a few runs (timeouts), never with
  `--test-threads=2`; Go's suite has the same retry for the same reason.
- The oracle's disk edits modify fixture files in place and revert them; an interrupted run leaves a fixture edited.

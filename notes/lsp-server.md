# lsp-server: `tsrs_lsp` (Go `internal/lsp`) and `tsrs --lsp -stdio`

Wave `server` of the language-server port: `server.rs`, `logger.rs`, `dynamic_queue.rs`, `progress.rs`,
`stack_sanitizer.rs`, `workerpool.rs` in `crates/tsrs_lsp`, and `crates/tsrs_cli/src/lsp.rs`. Every Go handler is
registered with the same registration kind and in Go's order (code action registered twice, the auto-import variant
wins, like Go). This note keeps the threading details and the deviations that still hold.

## Threading (docs/LSP.md "Threading model")

- `Server::run`: Go's errgroup -> `errGroup` (first error cancels the group context; `wait` joins). Dispatch and
  write loops are group threads; the read loop runs on a detached thread that reports its result to the group (Go's
  extra group member waiting on `readLoopErr` / `ctx.Done()` folded into it). All three have 512 MB stacks (the
  dispatch thread builds programs for synchronous handlers).
- Async request work (`go func() { doAsyncWork() }()`) runs on a process-wide pool of at most
  `max(4, available_parallelism)` threads with 512 MB stacks, started on demand, FIFO, never a thread per request.
  Difference to Go: at most that many async jobs run concurrently; a job blocked in `sendClientRequest` holds its
  thread.
- `dynamicQueue` = `Mutex<VecDeque>` + `Condvar`; waits are cancelable through the context (`wait_until`: an
  `after_func` on the context notifies the condvar, so cancellation wakes waiters without polling).
- `pendingServerRequests` channels -> `responseChan` (mutex + condvar, same select-on-ctx semantics).
- `$/cancelRequest` cancels the request's `Context` through `pendingClientRequests`, exactly like Go.
- Panics: `defer s.recover(req)` -> `Server::with_recover` (`catch_unwind`; a panic hook records the backtrace at
  the panic site while inside a recover scope, instead of printing). Unrecovered panics on any server thread or
  worker exit the process with status 2, like an unrecovered goroutine panic.
- `initComplete` (closed channel) -> a `Context` canceled when initialization completes.

## Deviations

- Out of scope (docs/LSP.md): content mappers (`Spawn`, `contentMapperSpawner`, `contentMapperLogger`,
  `custom/setContentMapperContributions`), `api` sessions (`custom/initializeAPISession`), pprof requests
  (`custom/runGC`, heap/alloc/CPU profiles). The handlers are registered like Go and answer through `not_yet_ported`
  (MethodNotFound "<method> is not ported yet"); no other method does.
- "Resolved client capabilities" log line: Go marshals the resolved capabilities as JSON; the `Resolved*` views have
  no codec, so the line shows their Debug form (log only; the oracle drops `window/logMessage`).
- Stack traces are Rust backtraces; `sanitizeStackTrace` looks for Go frames and returns "" for them (only used
  for telemetry, which is out of scope).
- CLI: no signal handling (Go's `signal.NotifyContext` cancels the server on SIGINT/SIGTERM, which makes `Run`
  print "context canceled" and exit 1; tsrs keeps the default signal action). `-pprofDir` prints Go's message but
  writes no profile. Windows `isProcessAlive` is not ported (watchdog disabled there).
- `locale.Parse`: English only; any non-empty tag is accepted and kept as text.
- `textDocument/diagnostic` with `trackFlakyDiagnostics`: Go runs `Program.Emit` (writing nothing) between the two
  diagnostics passes; the port does not call `Program::emit` there, so the passes run back to back (the
  comparison/logging/panic logic is Go's).
- The cross-project orchestrator's `GetLanguageServiceForProjectWithFile` finds the project by id in the current
  snapshot (Go type-asserts `ls.Project` to `*project.Project`; the trait has no downcast). The session looks the
  project up by id in a fresh snapshot either way. `GetProjectsLoadingProjectTree` collects the projects while the
  snapshot is held (Go yields lazily).
- Requests reaching a language-service handler before `initialized` dereference the missing session and end the
  process, as Go's nil dereference does.
- The synctest fake clock of `progress_test.go` is real time plus a `wait_idle` helper (Go's `synctest.Wait`).
- json/v2's max nesting depth (10000) is checked before encoding outgoing messages (`lspWriter`), so an
  unserializable response fails only its request (Go test `TestWriteLoopRecoversFromUnserializableResponse`).

Shutdown/exit: like tsgo-ref, `exit` ends the process with status 1 and "context canceled" on stderr (Go's `Run`
returns the canceled group error); `tools/oracle/lsp/exit_check.py` compares the combinations.

Status (2026-10-10): the builtin watcher and all phase-3 handlers this note once listed as "not yet ported" are
ported (notes/lsp-robust.md; docs/LSP.md, Progress).

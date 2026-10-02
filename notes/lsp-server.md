# lsp-server: `tsrs_lsp` (Go `internal/lsp`) and `tsrs --lsp -stdio`

Wave agent `server`, branch `lsp-server`.

## Ported

| Go (`ts-ref/tsc/…`) | Rust |
| --- | --- |
| `internal/lsp/server.go` | `crates/tsrs_lsp/src/server.rs` |
| `internal/lsp/logger.go` | `crates/tsrs_lsp/src/logger.rs` |
| `internal/lsp/dynamic_queue.go` (+ test) | `crates/tsrs_lsp/src/dynamic_queue.rs` (+ `dynamic_queue_test.rs`) |
| `internal/lsp/progress.go` (+ test) | `crates/tsrs_lsp/src/progress.rs` (+ `progress_test.rs`) |
| `internal/lsp/stack_sanitizer.go` (+ test) | `crates/tsrs_lsp/src/stack_sanitizer.rs` (+ `stack_sanitizer_test.rs`, compares the reference baselines) |
| `internal/lsp/server_test.go` | `crates/tsrs_lsp/src/server_test.rs` |
| `internal/lsp/server_projectinfo_test.go`, `server_progress_test.go`, `server_flakydiagnostics_test.go` | `crates/tsrs_lsp/src/server_{projectinfo,progress,flakydiagnostics}_test.rs` |
| `testutil/lsptestutil/lspclient.go` (in-process client over framed byte pipes) | `crates/tsrs_lsp/src/lsptestutil.rs` (test module) |
| `ls/completions.go`, `ls/signaturehelp.go`, `ls/semantictokens.go`: trigger characters, `SemanticTokensLegend` | `crates/tsrs_lsp/src/lsconsts.rs` (move to `tsrs_ls` when those files are ported in phase 3) |
| `cmd/tsc/lsp.go`, `isprocessalive_unix.go`, `isprocessalive_other.go`, the `--lsp` case of `cmd/tsc/main.go`, `osvfs.GetGlobalTypingsCacheLocation` | `crates/tsrs_cli/src/lsp.rs`, `crates/tsrs_cli/src/main.rs` |
| goroutines for async request work | `crates/tsrs_lsp/src/workerpool.rs` (see Threading) |

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

## Shared-file edits

- `crates/tsrs_lsproto/src/error.rs`: `ErrorTag::ContextCanceled`, `ErrorTag::ContextDeadlineExceeded`,
  `ErrorTag::Sentinel(&'static str)` (package-level sentinel errors / error types for `errors.Is` / `errors.AsType`),
  `impl From<ContextError> for Error`.
- `crates/tsrs_cli/Cargo.toml`: depends on `tsrs_lsp`.

## Deviations

- File watching: `lspwatcher` (the builtin in-process watcher) is phase 4. `handleInitialized` keeps Go's three-way
  decision; when the client lacks dynamic watch registration it takes the "disabled" branch with a log line saying
  the builtin watcher is not ported (Go would use FSEvents on macOS / Windows). `builtinWatcher` is an uninhabited
  type, so every builtin-watcher branch keeps Go's structure and is statically unreachable.
- Content mappers (`Spawn`, `contentMapperSpawner`, `contentMapperLogger`, `custom/setContentMapperContributions`
  parsing), `api` sessions (`custom/initializeAPISession`), pprof requests (`custom/runGC`, heap/alloc/CPU profiles)
  are out of scope (docs/LSP.md): the handlers are registered like Go and answer through `not_yet_ported`.
- "Resolved client capabilities" log line: Go marshals the resolved capabilities as JSON; the `Resolved*` views have
  no codec, so the line shows their Debug form (log only; the oracle drops `window/logMessage`).
- Stack traces are Rust backtraces; `sanitizeStackTrace` looks for Go frames and returns "" for them (only used
  for telemetry, which is out of scope).
- CLI: no signal handling (Go's `signal.NotifyContext` cancels the server on SIGINT/SIGTERM, which makes `Run`
  print "context canceled" and exit 1; tsrs keeps the default signal action). `-pprofDir` prints Go's message but
  writes no profile. Windows `isProcessAlive` is not ported (watchdog disabled there).
- `locale.Parse`: English only; any non-empty tag is accepted and kept as text.
- `textDocument/diagnostic` with `trackFlakyDiagnostics`: Go runs `Program.Emit` between the two diagnostics
  passes; emit is not ported, so the passes run back to back (the comparison/logging/panic logic is Go's).
- The cross-project orchestrator's `GetLanguageServiceForProjectWithFile` finds the project by id in the current
  snapshot (Go type-asserts `ls.Project` to `*project.Project`; the trait has no downcast). The session looks the
  project up by id in a fresh snapshot either way. `GetProjectsLoadingProjectTree` collects the projects while the
  snapshot is held (Go yields lazily).
- Requests reaching a language-service handler before `initialized` dereference the missing session and end the
  process, as Go's nil dereference does.
- Go tests not ported: `server_completion_test.go`, `server_semantictokens_test.go`,
  `server_projectreference_updates_test.go` (phase-3 features), `server_contentmapper*_test.go` (content mappers),
  `replay_test.go` (replays recorded sessions through fourslash-style tooling; needs `-replay` input). The synctest
  fake clock of `progress_test.go` is real time plus a `wait_idle` helper (Go's `synctest.Wait`).
- json/v2's max nesting depth (10000) is checked before encoding outgoing messages (`lspWriter`), so an
  unserializable response fails only its request (Go test `TestWriteLoopRecoversFromUnserializableResponse`).

## Handler table

Every Go handler is registered with the same registration kind (`registerRequestHandler`,
`registerNotificationHandler`, `registerLanguageServiceDocumentRequestHandler`,
`registerLanguageServiceWithAutoImportsRequestHandler`, `registerMultiProjectReferenceRequestHandler`), in Go's order
(code action registered twice, the auto-import variant wins, like Go). Ported end to end: initialize, initialized,
shutdown, exit, didChangeConfiguration, didOpen/didChange/didSave/didClose, didChangeWatchedFiles, $/setTrace,
custom/setLogVerbosity, textDocument/diagnostic, hover, definition, typeDefinition, custom/textDocument/sourceDefinition,
custom/projectInfo; the server's `project.Client`, `logging.Logger` and `ata.NpmExecutor` implementations.

### Not yet ported (answered by `not_yet_ported(method)`: MethodNotFound "<method> is not ported yet")

Phase 3 (the `ls` function does not exist yet; the registration, language-service acquisition and async work are
already Go's): signatureHelp, formatting, rangeFormatting, onTypeFormatting, documentSymbol, documentHighlight,
custom/textDocument/multiDocumentHighlight, selectionRange, inlayHint, codeLens, codeAction, prepareCallHierarchy,
foldingRange, prepareRename, linkedEditingRange, completion, _vs_onAutoInsert, references, _vs_references, rename,
implementation, callHierarchy/incomingCalls, callHierarchy/outgoingCalls, workspace/symbol, completionItem/resolve,
codeLens/resolve, semanticTokens/full, semanticTokens/range, workspace/willRenameFiles (after Go's empty-input
checks).

Out of scope (docs/LSP.md): custom/runGC, custom/saveHeapProfile, custom/saveAllocProfile, custom/startCPUProfile,
custom/stopCPUProfile (pprof), custom/initializeAPISession (api), custom/setContentMapperContributions (content
mappers; `parseContentMapperContributions` not ported).

## Needs from others

- `tsrs_ls`: move `lsconsts.rs` contents into `completions.rs` / `signaturehelp.rs` / `semantictokens.rs` when ported.

## Doubts

## Gates

- `cargo check --workspace`: 0 errors, 0 warnings. `cargo test -p tsrs_lsp`: 26 passed.
- `python3 tools/oracle/lsp/lsp_oracle.py --conformance genericRestParameters1 asyncAwait_es6 --edits`: initialize 2/2,
  diagnostic 6/6, hover 240/240, definition 240/240, publishDiagnostics 2/2 equal.
- Same with `--requests diagnostic,hover,definition,typeDefinition` on moduleResolutionWithExtensions,
  importsImplicitlyReadonly, declarationEmitPromise, exportAssignmentMembersVisibleInAugmentation,
  typeGuardsInClassMethods (`--positions 20 --edits`): all 1204 requests equal; also on assignmentCompatability10,
  classAbstractInstantiations1 (diagnostics with errors).
- Shutdown/exit: like tsgo-ref, `exit` ends the process with status 1 and "context canceled" on stderr (Go's `Run`
  returns the canceled group error).

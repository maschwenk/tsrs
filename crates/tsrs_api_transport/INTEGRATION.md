# Runtime integration review for core candidate a9b4354

This review covers the core candidate `a9b435432f4fa38e140fccacc50d461d15e4ce90` (`mfs-cx/node-api-core`), which
contains this crate at fe051ba. The reference is the pinned Go server built at b85298b (`go build ./cmd/tsc`).
Both servers were driven by the pinned Node clients.
Core and the checker lane own every change listed below; this crate does not edit their files.

## Callback re-entry (tests/node/reentry_repro.mjs)

Each scenario runs in its own child process, killed after 20 s. The outer request is `createSnapshot`
of a 50-file project, or `build` for the `*DuringBuild` rows. While the server reads `b.ts` through the
`readFile` callback, the client makes the nested request listed below, in both sync and async modes.

| nested request | pinned Go | tsrs a9b4354 |
| --- | --- | --- |
| ping, parseConfigFile, createSnapshot (other project) | ok | ok |
| getSemanticDiagnostics on an earlier snapshot | ok | ok |
| updateSnapshot / release of an earlier snapshot | ok | ok |
| createSnapshot of the same project, or of the in-flight project / file | ok | ok |
| ping during an orchestrator build | ok | ok |
| build on the orchestrator that is building | **hang** (killed) | deliberate `api: client error: the build orchestrator is in use…` |

Result: 22 of 22 scenarios finish under tsrs. In tsrs_project, no snapshot operation hangs when it
re-enters, including parsing the in-flight project from several worker threads.

Core today guards one lock (`build.rs` build orchestrators) with `lock_for_request`.
The other session locks are all short-lived and never held across filesystem access (`snapshots`,
`batch_pages`, `source_files.*`, `module_resolvers.*`). Keep it that way. Any lock or exclusive lease that is
held while the program or config is read through the session filesystem must use
`tsrs_api_transport::lock_for_request`, or check `blocking_may_deadlock()` and return
`reentrancy::reentrancy_error(..)`. Examples: a future languageServerUpdateMu equivalent, a module-resolver
context held during resolution, or the checker lease (owned by the checker lane).

## Close and cancel (tests/node/close_repro.mjs)

The client disconnects while the server waits on a `readFile` callback it will never get an answer to.

| case | pinned Go | tsrs a9b4354 |
| --- | --- | --- |
| stdin EOF (sync, async) | exit 0, under 2 ms | exit 0, under 2 ms. stderr carried a panic report; fixed in this crate (see below) |
| client vanishes (stdin and stdout closed) | killed by SIGPIPE | exit 1, `ipc: failed to write response: Broken pipe` |

The fix: a callback failure now unwinds with `resume_unwind`, so no panic report reaches stderr, which
matches Go's silent recover. I re-ran the repro against a9b4354 with only `src/callbackfs.rs` replaced;
stderr is empty for EOF. Genuine handler panics are still reported.

`SessionHandler` (in `crates/tsrs_cli/src/api.rs`) does not read `RequestContext.cancel`. After EOF, a
long request that makes no callbacks runs to completion before `run` returns and joins it. Go behaves the
same unless its project code observes ctx.

## Strict JSON for sync request params (core: `session.rs` `parse_params`, line ~406)

Async requests are validated by the transport (`jsonrpc::decode_message`). In Go, a violation there is
fatal to the connection, and tsrs matches. Sync (MessagePack) params reach core as raw bytes and are parsed
with `tsrs_core::json`. That parser rejects duplicate keys but accepts unpaired surrogate escapes. Probes
against both binaries:

| sync request | pinned Go | tsrs a9b4354 |
| --- | --- | --- |
| `createSnapshot` with file content `"x\ud800y"` | `api: invalid request: failed to unmarshal *api.CreateSnapshotParams: jsontext: invalid surrogate pair …` | accepted |
| duplicate `/p/a.ts` in `fileSystem.files` | `… jsontext: duplicate object member name "/p/a.ts" within "/fileSystem/files"` | rejected, text `… at offset 65` |
| `parseConfigFile` with `"/p/\udc00.json"` | invalid request (surrogate) | `api: client error: could not read file "/p/\u{fffd}.json"` |
| `echo "x\ud800"` | echoes the raw bytes | same |

Plan for core: in `parse_params`, for every method except `echo` and `ping`, call
`tsrs_api_transport::strictjson::validate(params)` before `json::unmarshal`. On error, return
`ApiError::invalid_request(format!("failed to unmarshal *api.{GoParamsType}: {e}"))`.
Go's text is `api: invalid request: failed to unmarshal *api.<Type>Params: <jsontext error>`, and
`strictjson` already produces that jsontext error text exactly (tests/strictjson_oracle.rs).

## Request filesystem

Core keeps its own `crates/tsrs_api/src/requestfs.rs` instead of this crate's `requestfs` feature.
Its filesystem behaviour matches pinned Go on this crate's differential fixture: I ran core's
`RequestParams::parse` + `new_for_update` over `tests/fixtures/requestfs_scenarios.json` and
`requestfs_go.json` with a scratch test that I did not commit. All 242 comparisons (FS queries plus per-step file-change summaries) agree. Structural
differences from Go that the fixture does not reach:

- The base is the host FS (`Arc<dyn FS>`), not the snapshot's layered or overlay FS. There is no
  `overlays()`, no `FileHandleSource`, no `FileChangeExpander`, and no rebasing in tsrs_project's
  `layer_overlay_file_system`. Go uses these for overlays under request layers (language-server
  snapshots) and to expand `fileNotifications` to symlink aliases. I could not demonstrate a visible
  difference: an alias-invalidation probe showed no diagnostics change in either Go or tsrs, so it was
  inconclusive.
- `add_file_changes` ignores the "overlay dropped by the new layer" branch (`_fs` is unused).

Plan: either switch core to `tsrs_api_transport::requestfs` (`AnyFs`, `RequestFs`; it implements
`LayeredFileSystem`, `FileChangeExpander` and `base_file_system`/`with_base_file_system`), or port
those trait impls into core's type. Then add the rebasable hook in `tsrs_project::layer_overlay_file_system`
(Go `overlayfs.go:226`).

## Not supported (explicit)

Windows named pipes (`--pipe` on non-Unix returns Unsupported) and external content mappers.

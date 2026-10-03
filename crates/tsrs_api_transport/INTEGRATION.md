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

# Review of core d6d8e2a (PR 34)

**Inputs**
- Core checkout: `d6d8e2a540cf18feb5d03286ae069968a701bee4`, in an isolated worktree.
  - `target/debug/tsrs` sha256 `9d65e458…6304`. A clean rebuild of the same head produced `5e114214…5883`; debug builds are not byte-reproducible.
- Pinned Go: `go build ./cmd/tsc` at microsoft/TypeScript `b85298b6a81f772d080b0455de0ca9d744cd6fd6`, sha256 `98159140…eacc`.
- Clients:
  - pinned `packages/typescript/src/api` at the same commit;
  - the SDK at `npm/tsrs/src/api` as of 1387320 (from d6).
- Harness: the scripts in `tests/node/`.

## Strict request JSON: matches, with a gap in type errors

Probe: all 172 pinned methods, sync, with malformed params, against both binaries. Per-method results are
in the `d6-strict-probe-{go,tsrs}.json` artifacts.

| params | result |
| --- | --- |
| lone surrogate (`{"snapshot":"\ud800"}`) | 171/172 identical text |
| top-level duplicate | 171/172 identical |
| nested duplicate (`{"x":{"y":[{"z":1,"z":2}]}}`) | 171/172 identical |
| invalid UTF-8 | 171/172 identical |
| truncated JSON | 171/172 identical |

The one difference is `stopCPUProfile`, which is explicitly unsupported.

Remaining gaps (core: `session.rs` `parse_params` / `wire.rs`):

- **Wrong top-level type** (`[1]`): Go rejects all 171 methods that take params with
  `api: invalid request: failed to unmarshal *api.<T>: json: cannot unmarshal JSON array into Go api.<T>`.
  tsrs:
  - 121 methods answer `invalid request`, but with other text;
  - 37 answer `client error`;
  - 5 succeed (`createBuildOrchestrator` creates an orchestrator; also `parseCommandLine`,
    `createSourceFile`, `transpileModule`, `transpileDeclaration`);
  - 5 return untyped `build orchestrator not found` errors.

  Fix: after `strictjson::validate`, reject any non-object, non-null params for methods that
  take params, using Go's text.
- **Wrong field types** (`{"snapshot":"x","project":7,...}`):
  - 148 methods are `invalid request` in both, but the text differs. Go says
    `json: cannot unmarshal JSON string into Go api.SnapshotID within "/snapshot"`; tsrs says
    `snapshot must be a non-negative integer`.
  - 3 methods differ in class (Go `invalid request`, tsrs `client error`): `retainSourceFile`
    and two others.

## Callback re-entry and EOF on d6: unchanged and bounded

- `reentry_repro.mjs`: all 22 scenarios DONE; nested snapshot operations succeed as in Go.
- `buildDuringBuild` returns the deliberate error, where Go hangs.
- `close_repro.mjs`:
  - EOF while a callback is pending: exit 0 within 2 ms, stderr empty;
  - client gone: exit 1, `Broken pipe`.

## Async false positives in the re-entrancy gates: found and fixed in the transport

`tests/node/contention_repro.mjs` sets up this situation: request C waits on a slow client callback
without holding anything, while requests A and B contend for the same orchestrator or API checker.
Nothing re-enters, and Go serves everything.

| resource, 3 rounds | pinned Go | d6 | d6 + this fix |
| --- | --- | --- | --- |
| build orchestrator | 0 errors | 6 spurious `…is in use by a request that is waiting on a client callback` | 0 |
| API checker (checker lane's `lease.rs`) | 0 | 2 spurious | 0 |

Cause: `blocking_may_deadlock()` treated any request that was waiting on the client, anywhere on the
connection, as proof of re-entry.

Fix: `reentrancy.rs`; no change to the signature or call sites.
- **Sync** is unchanged. A conflict there is provably a nested re-entry, so it still fails immediately.
- **Async** waits out a grace period before failing: `ConnOptions` / `ServeOptions::reentrancy_grace`,
  default 10 s. A real async re-entry deadlock now ends with the same error after the grace period
  instead of failing early on valid concurrency.

Verification:
- I applied the change to a local copy of d6 and rebuilt; I did not commit it to core.
- `contention_repro.mjs`: 0 errors for both resources.
- `reentry_repro.mjs`: async `buildDuringBuild` errors after 10.3 s; the other re-entry scenarios are unchanged.
- The checker lane's `session_tests::lease_reentrancy` tests: 3/3 pass, in 10 s instead of instantly. They
  can set a short `reentrancy_grace` in their `ConnOptions` if they want the old speed.
- New transport test: `async_unrelated_slow_callback_does_not_reject_ordinary_contention`.

## Checker lease gate (270978d)

`lease.rs` takes a per-program gate before the API checker slot and uses `blocking_may_deadlock()`.
- Lock order, gates map before gate, is consistent between acquire, drop and forget. No inversion.
- Entries are removed when unused.
- The address-keyed map is only used while the program is alive.

Its async false positive came from the transport predicate and is fixed above. No checker-file change
is required.

## SDK connection-loss patch (1387320)

`tests/node/connloss_repro.mjs` checks, for each case, that every promise settles and that there is no
`uncaughtException` or `unhandledRejection`. Each case runs in its own child process, killed after 20 s.

Results with the SDK at d6, against both tsrs d6 and Go:
- All 5 cases settle (kill during callback, close after death, kill-then-request, close during callback,
  kill during batch).
- No uncaught exceptions or unhandled rejections.
- `close()` resolves.

The unpatched pinned client against tsrs d6 leaves three cases unsettled (process exit 13): kill during
callback, kill-then-request, and kill during batch. So the patch is needed.

Cosmetic differences:
- `close()` during a callback rejects the in-flight request with vscode-jsonrpc's
  `Pending response rejected since connection got disposed` rather than the patch's error. This is
  deliberate: it was the client's own close.
- The detail text depends on which signal arrives first (EPIPE vs closed).

## Request filesystem alias gap: concrete repro and bounded fix plan (core)

`tests/node/requestfs_alias_repro.mjs`:
- A request layer adds symlink `p/link -> p/target` (a host directory). The project compiles `link/a.ts`.
- The host file `p/target/a.ts` is fixed on disk, and `updateSnapshot` notifies the change at the
  target path only.

| semantic diagnostics before → after, by notification | target path | alias path | none |
| --- | --- | --- | --- |
| pinned Go | 1 → 0 | 1 → 0 | 1 → 1 |
| tsrs d6 | **1 → 1 (stale)** | 1 → 0 | 1 → 1 |

Go expands notifications to request-symlink aliases in `snapshot.processFileChanges`
(`FileChangeExpander`). Core passes its request filesystem to tsrs_project as a plain `FS`, so the
expansion never runs.

Fix, about 18 lines (artifact `d6-core-alias-expansion-proposal.patch`):
1. Add `pub fn expand_file_changes(&self, FileChangeSummary) -> FileChangeSummary` to core's
   `RequestFileSystem`. It is a port of `filechanges.go` `ExpandFileChanges` and uses the existing
   `aliases_for_path`.
2. In `snapshots.rs`, apply it to `file_changes` with the new snapshot's request FS just before
   `clone_snapshot`. The patch covers update; do the same in `handle_create_snapshot`.

With this applied locally, tsrs gives `target: 1 → 0`, matching Go.

Overlay rebasing and the "overlay dropped" branch are only reachable from LSP-connected API sessions.
`getCurrentLanguageServerSnapshot` returns `requires an LSP-connected API session` in both Go and tsrs
for `--api`, so standalone `--api` cannot observe them. That remains a documented gap.

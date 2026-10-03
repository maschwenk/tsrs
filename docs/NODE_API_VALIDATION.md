# Node API validation (parity lane)

How the `tsrs --api` server is validated against the pinned TypeScript 7 native API
(microsoft/TypeScript `b85298b6a81f772d080b0455de0ca9d744cd6fd6`, the commit in `Cargo.toml`). The
contract itself is `docs/NODE_API.md`; this file covers the independent evidence.

## Principles

- The oracle is the Go server built from `ts-ref/tsc` at the pinned commit (`go build ./cmd/tsc`), not an
  npm package.
- The client is the pinned upstream client (`ts-ref/packages/typescript/src/api`), unmodified. The harness
  never ships a second client; the shipped SDK (`npm/tsrs`) is a byte-for-byte copy checked by
  `node npm/sdk/sync-upstream.mjs --check`, and CI separately exercises the packed SDK.
- Tests are the upstream suites (`packages/typescript/test/{sync,async}`) plus harness tests in
  `tools/node-api/tests`, all run by `node --test` against real server processes. Every harness test
  must pass on the Go oracle before it can be used against tsrs.
- Results are judged from saved per-test results and real counts, never from an exit code or from
  which methods a client exports. Failures are reported to the owning lane; the harness is never edited
  to hide them.

## Running

```sh
tools/node-api/setup.sh                       # ts-ref at the pin, npm ci, Go oracle -> tools/node-api/.work/oracle/tsc
node tools/node-api/run-upstream.mjs --binary tools/node-api/.work/oracle/tsc --label go-oracle
cargo build --release -p tsrs_cli
node tools/node-api/run-upstream.mjs --binary target/release/tsrs --label tsrs
node tools/node-api/inventory.mjs --oracle go-oracle --candidate tsrs --md /tmp/inventory.md --json /tmp/inventory.json
```

`run-upstream.mjs` copies the pinned client and tests into `tools/node-api/.work/<label>/tree`, points the
client's `built/local/tsc` at the chosen binary and writes `results.jsonl` (one line per test, full suite
path), `run.tap`, `summary.json` and `trace/*.jsonl`. Options: `--suite upstream|parity|all`,
`--filter <path segment>`, `--no-trace`, `--timeout-min N` (default 30), `--idle-sec N` (default 180) and
`--test-timeout-sec N` (default 120).

`--filter` matches path segments, not raw substrings. `sync/api.test` selects only `test/sync/api.test.ts`,
`sync` selects the whole directory, and `api` or `api.test` selects that basename in every directory. A
filter that matches no file is an error.

The run is bounded three ways:

- `--test-timeout` fails an individual hung async test.
- The idle watchdog kills the run's whole process group when no new result appears for `--idle-sec`. This
  covers a test blocked in a synchronous read, or a file process that never exits.
- `--timeout-min` caps the total run time.

Results gathered before a kill are kept. `summary.json` records `timedOut` (`idle` or `total`), the leaked
processes found in the group (test files or API servers still alive after `node --test`, which are then
killed), and `filesWithoutResults`. Any timeout or leak makes the run exit 1. The runner prints a progress
line to stderr every 30s.

### Tracing and per-method inventory

On Linux the binary is wrapped by `proxy.mjs`, a byte-transparent tap that records frame metadata only
(method, direction, size, error text). It parses sync MessagePack tuples and async `Content-Length`
JSON-RPC, including method names inside `batchRequests` and per-entry batch errors. `preload.mjs` wraps
`node:test` so each spawned server records the full name of the test that started it.

`inventory.mjs` reads the 172 `Method` constants from the pinned `proto.go` and, per method, reports
oracle and candidate sends, answers, errors, explicit unsupported errors, and the tests that sent it with
their outcome. Candidate status is one of:

- `unexercised`: no test sends it.
- `not-sent`: the oracle sends it, the candidate run did not.
- `unimplemented`: only explicit unsupported errors.
- `failing`: a non-todo test that sent it failed, or it errored more often than on the oracle.
- `error-only-unverified`: both servers only returned errors; equal counts are not proof the errors are
  equivalent.
- `tests-pass`: every test that sent it passed under its own assertions.

`tests-pass` is not response-level parity. Error-text differences between the two runs are listed per
method regardless of status. Traffic from todo tests (known upstream defects) is excluded from the
per-method counts and listed separately. Owner and claimed status come from `crates/tsrs_api/src/methods.rs`
when present and are shown next to the measured status.

### Parity tests and goldens

`tools/node-api/tests/*.test.ts` use the pinned client like the upstream tests do and compare results with
`tools/node-api/tests/golden/*.json`. Goldens are recorded only from the Go oracle
(`run-upstream.mjs --suite parity --record`, which refuses any other binary). Covered so far: UTF-16
positions after astral/BMP text, diagnostic chains and related info, config parsing, global and program
diagnostics, `extends` through node_modules, package `exports`, root-file programs, and emit
(`noEmit`, `noEmitOnError`, `emitDeclarationOnly` with declaration maps, `allowJs` declarations, source
maps with inline sources, d.ts inputs, `emitToString` / `getJavaScriptEmit` / `getDeclarationEmit`).

More gates, all oracle-recorded:

- Snapshots during edits (`snapshot.test.ts`): the old snapshot keeps its diagnostics, types and file lists
  across change, create and delete, including after the newer snapshot is disposed. An update without
  `ensurePrograms` stays lazy, which is the pinned semantics.
- Handles (`lifecycle.test.ts`): released, invalid and foreign snapshot/project/symbol/node handles, invalid
  params and unknown methods are sent through the client's own request path. The server must keep serving
  after each rejection. Error wording is compared softly.
- Callbacks and lifecycle (`lifecycle.test.ts`, each case in a subprocess with a hard timeout): re-entrant
  requests from fs callbacks, async close with 50 requests in flight, and SIGKILL of the server under both
  bindings.
- Malformed framing (`transport.test.ts`): 10 sync and 10 async raw byte streams sent to the binary with no
  client. Each must exit within 8s of EOF, and stdout may only ever carry whole frames.
- Memory (`rss.test.ts`, Linux): 200 edit/ensure/query/dispose cycles each via `update` and via
  `createSnapshot`. RSS after warm-up is bounded, and growth in the last third must not exceed growth in the
  first. Samples go to `.work/<label>/evidence/rss-*.json`.
- Project references and buildinfo (`build.test.ts`): a no-op rebuild, up-to-date detection from buildinfo
  in a new session, buildinfo shape, and a referenced project's breaking change surfacing downstream.
- AST and checker at the UTF-16/WTF-8 boundary (`ast.test.ts`): full TSX traversal with astral identifiers
  and lone-surrogate escapes, 400-deep nesting, checker queries by UTF-16 position, overloads and JSDoc JS.
- Methods no upstream test sends (`gaps.test.ts`): `getExportSymbolOfSymbol`,
  `getOuterTypeParametersOfType`, `getConstraintOfType`, `getCurrentLanguageServerSnapshot` and the profiling
  methods.

### Upstream defects found (tests marked todo, reported as failing, never counted as passes)

1. Pinned Go server panics (`jsontext: invalid surrogate pair`) and exits when a readFile callback returns
   text with a lone surrogate.
2. Pinned Go server intermittently drops the file being read when a nested sync request runs inside its
   readFile callback: 3/40 runs, against 0/40 with no nested request and 0/40 with the async binding.
3. Pinned async client never settles in-flight requests after the server process dies. Node exits 13 with
   an unsettled top-level await.

Also observed, but not marked todo: after a server crash the sync client's `close()` throws `EBADF`.

### Installed package and CI

`tools/node-api/consumer.mjs --dist <pack dir>` installs the `npm/build.mjs --pack` tarballs offline into a
temp project outside the repo. It then runs the installed binary through CLI pass/fail regressions, sync
and async API programs on the OS filesystem (diagnostics plus emit with `TSRS_EMIT` unset), and a nodenext
typecheck of the published declarations that must report exactly one deliberate error.

`.github/workflows/node-api.yml` runs on main pushes, manual dispatch, and `pull_request` into main for
API paths. It never uses `pull_request_target`, has read-only permissions and no secrets, and never
publishes. Manual dispatch only works once the file is on the default branch, so pull requests are how the
API branches get validated before a merge. The `parity` job runs the Go oracle (must pass), then tsrs
against the same suites, then the inventory, uploads the results, and fails on any tsrs failure. The
`package` job builds, packs and runs `consumer.mjs` on linux-x64, linux-arm64 and darwin-arm64.

## Current evidence

Go oracle at the pin, Linux x64, Node 24.21: upstream sync+async suites 842/842 tests passed (7 files,
787 server processes, 0 unparseable frames). 165 of 172 methods are sent by at least one upstream test.
Not sent by any upstream test: `getCurrentLanguageServerSnapshot`, `getExportSymbolOfSymbol`,
`getOuterTypeParametersOfType`, `getConstraintOfType`, `startCPUProfile`, `stopCPUProfile`,
`saveHeapProfile`. These need harness tests before any claim (the profiling methods are Go-runtime
specific).

Full run on the Go oracle (upstream plus parity, traced): 893 tests, 891 passed, 0 failed, 2 todo, 820
server processes, 172/172 methods sent by at least one test. The 2 todo tests are upstream defects 1 and 3.
Defect 2's 30-iteration loop test, added afterwards, is a third todo. With it, the parity suite alone
(`--no-trace`) gave 49 passed, 0 failed, 3 todo in each of 3 consecutive runs. Earlier parity-only result:
12/12. Consumer check with the SDK branch's packaging (`d97535a`) and
the Go oracle as the packed binary: 18/18 checks. That validates the harness and packaging path only,
not Rust.

Runtime lane's test server (`mfs-cx/node-api-runtime` `fe051ba`,
`examples/transport_test_server`, release build): malformed-framing gates 20/20, with protocol-clean stdout
and bounded exits. There were 2 soft differences from Go, and they are stricter than Go: a huge declared
sync payload or async `Content-Length` is rejected before EOF, where Go fails only at EOF. That server is a
transport fixture, not the API session, so this says nothing about method parity.

tsrs at core `a9b435432f4f`, release build, measured with harness `ad6a61b`. That harness has the same tests
and goldens as the commit that adds this text; the later change is inventory reporting only.

- Results: 902 tests, 897 passed, 2 failed, 3 todo. 0 timeouts, 0 leaks, 0 skipped suites. A no-trace
  rerun gave the same results.
- Upstream suites:
  - sync/api 339/339, async/api 347/348;
  - ast 111/111, generators 43/43, shutdown 1/1;
  - astnav sync+async 8/8, each checking 9772 positions.
- Parity tests: 46/48.
- Failures:
  - RSS grows about 3 MiB per `createSnapshot`+dispose cycle. The update path plateaus.
  - The upstream batchContext test expects a Go `panic:` in `getTypeArguments`; tsrs returns an explicit
    client error instead.
- Inventory over 172 methods:
  - `tests-pass` 158;
  - `failing` 10, all attributed to those 2 tests;
  - `error-only-unverified` 1 (`getCurrentLanguageServerSnapshot`);
  - `unimplemented` 3 (the profiling methods).
- Known upstream defects: tsrs passes todo 1 (lone surrogate: explicit error, keeps serving) and todo 2 (no
  dropped file in 30 runs). Todo 3 is a client defect and fails on both servers.

### tsrs at core `d6d8e2a540cf`

- Server: release build at that exact commit.
- Harness: commit `6bda959`, the same tests and goldens as the copy at that commit plus the RSS probe.
- Client: the unmodified pinned upstream client. The packaged SDK patches its async client for crashes, but
  the harness doesn't use the packaged SDK.

Results:

- Full suite, traced, two runs: 902 tests, 899 passed, 0 failed, 3 todo. No timeouts, leaks or skipped
  suites.
- Server exits: only the intentional SIGKILLs from the crash tests.
- `sync/api.test`: 339/339 in 6 consecutive runs.
- Parity lifecycle/rss/snapshot/build/gaps stress: 10 runs, no crashes, failures, timeouts or leaks.
- RSS probe, 120 cycles (MiB at start, then at the end):

| Variant | Go | tsrs `a9b4354` | tsrs `d6d8e2a` |
|---|---|---|---|
| create-plain | 44→61 | 111→525 | 110→174, flat from cycle 40 |
| update | 44→65 | flat 112–114 | flat 112 |
| transpile | 45→117 | 109→171 | 104→113 |

- Inventory over 172 methods:
  - `tests-pass` 168;
  - `error-only-unverified` 1 (`getCurrentLanguageServerSnapshot`);
  - `unimplemented` 3 (the profiling methods).

### tsrs at core `7c34965` and `f70371e`

`7c34965` (harness `cc303bd`):

- Full suite, twice: 902 tests, 899 passed, 0 failed, 3 todo.
- `sync/api` + shutdown: 40/40 runs at 340/340.
- Lifecycle stress: 40/40 runs clean.
- Build RSS probe: grows about 25.7 MiB per orchestrator create/build/dispose.

`f70371e` (harness `1731464`):

- Full suite, run 1: one failure from the RSS late-growth check (+22 MiB in the last third, +17 MiB in the
  first).
- Full suite, run 2: 903 tests, 900 passed, 0 failed, 3 todo.
- That RSS test re-run: 5/5 passed.
- `createSnapshot` probe: flat over 600 cycles.
- Build probe: flat at about 140 MiB over 400 builds.
- Inventory over 172 methods: `tests-pass` 168, `error-only-unverified` 1, `unimplemented` 3.

Pinned Go build semantics, recorded and matched by tsrs: through the API, `dry` still writes outputs,
`force` on an up-to-date graph writes nothing, and `stopBuildOnErrors` still builds downstream projects.

### Paired responses (compare-responses.mjs)

`run-upstream.mjs --capture` records every payload, hashed in full and stored up to 4 MiB. Run the oracle twice
and the candidate once, each under the same label and therefore the same work-tree path, renaming the result
directory after each run. Then run
`compare-responses.mjs --a <go-1> --b <candidate> --a2 <go-2>`.

How exchanges are paired:

- Server processes are paired by test file, test name and request-method signature.
- Sync exchanges are aligned by order and async exchanges by JSON-RPC id. `batchRequests` is expanded into
  its inner calls.

The only normalizations:

- Namespaced handle bijections (symbol, type and signature ids, file node IDs, counter suffixes in escaped
  names). These are checked across dependent calls.
- Go panic stacks are stripped.
- Profiling temp paths are replaced.

Where the oracle is nondeterministic, a candidate exchange that equals either oracle run counts as equal.
Order is ignored only where the two oracle runs themselves disagree on order.

Negative controls:

- `--drift <method>` changes one candidate response.
- `--drift-handle` breaks one symbol reference.

Every control tried so far was detected.

Result at `b2769b8`. The four categories are disjoint and cover all 172 methods:

- 161 equal: 7428 successful exchanges.
  - 160 have both sync and async successful pairs.
  - `getExportSymbolOfSymbol` has only a sync wire pair in the suites. Its async coverage is in
    `async-export-symbol.test.ts`, below.
- 1 error-only: `getCurrentLanguageServerSnapshot`.
- 3 unsupported: `startCPUProfile`, `stopCPUProfile`, `saveHeapProfile`.
- 7 differ:
  - `createSnapshot` and `getDefaultProjectForFile`: duplicate VS Code URI root (see below);
  - `updateSnapshot`: `changes:{}` where Go omits it;
  - `cleanBuild`: `filesDeleted`;
  - `getTypeAtLocation` and `getTypeFromTypeNode`: ObjectFlags `MembersResolved`;
  - `getTargetOfType`: `elementFlags:[]` where Go omits it.

VS Code URI roots (`probe-uri-filenames.ts`): 80 runs covering both binaries × sync/async × 4 input orders × 5
repetitions.

- Both servers report 5 `rootFiles`/`fileNames` for the 4 open documents, sorted by path, with one entry
  duplicated.
- Go duplicates a randomly varying document, in every input order and both modes. All 4 documents were seen
  duplicated.
- tsrs always duplicates the notebook cell, which is one of the variants Go produces.
- The distinct sets are equal. The duplicate is a pinned-Go defect that tsrs reproduces deterministically. It
  is not a candidate ordering bias, but the duplicate root is wrong on both sides.

`getExportSymbolOfSymbol` through async (`async-export-symbol.test.ts`):

- The public `Symbol.getExportSymbol()` answers from the async client's symbol cache.
- The same test also sends a real `getExportSymbolOfSymbol` request with a valid symbol handle through the
  same async connection, and checks that the answer is the cached export symbol.
- This passes on Go and on `b2769b8`, and the wire request appears in both traces.

## Known limitations

- Tracing needs Linux (`/proc`); other platforms run with `--no-trace` and only get test counts.
- Batched-method error attribution scans the first 1 MiB of each batch payload.
- Attribution is per test process: a test that both passes and shares a server with a failing test is
  attributed independently, but a method sent from a suite-level `before` hook is attributed to that hook.

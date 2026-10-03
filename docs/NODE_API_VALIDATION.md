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
their outcome. Candidate status is one of `unexercised` (no upstream test sends it), `not-sent`,
`unimplemented`, `passing` (every test that sent it passed) or `failing`. `passing` is per-test
evidence, not a full-parity proof. Owner and claimed status come from `crates/tsrs_api/src/methods.rs`
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

`.github/workflows/node-api.yml` runs on main pushes and manual dispatch only. It has read-only
permissions, no secrets and never publishes. The `parity` job runs the Go oracle (must pass), then tsrs
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

tsrs: no integrated `--api` candidate has been measured yet.

## Known limitations

- Tracing needs Linux (`/proc`); other platforms run with `--no-trace` and only get test counts.
- Batched-method error attribution scans the first 1 MiB of each batch payload.
- Attribution is per test process: a test that both passes and shares a server with a failing test is
  attributed independently, but a method sent from a suite-level `before` hook is attributed to that hook.

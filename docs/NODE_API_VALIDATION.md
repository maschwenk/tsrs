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
`--filter <substring>`, `--no-trace`, `--timeout-min N`.

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

## Current evidence

Go oracle at the pin, Linux x64, Node 24.21: upstream sync+async suites 842/842 tests passed (7 files,
787 server processes, 0 unparseable frames). 165 of 172 methods are sent by at least one upstream test.
Not sent by any upstream test: `getCurrentLanguageServerSnapshot`, `getExportSymbolOfSymbol`,
`getOuterTypeParametersOfType`, `getConstraintOfType`, `startCPUProfile`, `stopCPUProfile`,
`saveHeapProfile`. These need harness tests before any claim (the profiling methods are Go-runtime
specific).

tsrs: no integrated `--api` candidate has been measured yet.

## Known limitations

- Tracing needs Linux (`/proc`); other platforms run with `--no-trace` and only get test counts.
- Batched-method error attribution scans the first 1 MiB of each batch payload.
- Attribution is per test process: a test that both passes and shares a server with a failing test is
  attributed independently, but a method sent from a suite-level `before` hook is attributed to that hook.

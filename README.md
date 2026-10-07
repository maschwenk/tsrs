# tsrs

[![npm](https://img.shields.io/npm/v/@maschwenk/tsrs)](https://www.npmjs.com/package/@maschwenk/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

**tsrs is a Rust port of the TypeScript 7 compiler and language server.** It takes `tsc`'s flags and tsconfig and
reports the same diagnostics as the Go compiler it ports. On the vscode codebase, on a 64-vCPU machine, it type-checks
[17.7x faster and 2.8x more memory efficient than tsc 7, and 1.55x faster than `bun check`](#benchmark-tsrs-vs-tsgo-702-vs-bun-check).
On the TypeScript conformance suite it matches the reference on 13,458 of 13,462 error baselines and on all 12,779
`.types` and `.symbols` baselines, in Go-compatible mode ([evidence](#what-tsrs-is)).

[Quick start](#quick-start) · [How fast](#how-fast) · [vs bun check](#tsrs-against-bun-check-thread-for-thread) ·
[What it does and doesn't do](#what-it-does-and-doesnt-do) · [Language server](#language-server) · [Docs](#docs)

## Quick start

```sh
npx -y @maschwenk/tsrs -p path/to/project            # macOS arm64, Linux x64/arm64; emits like tsc
npx -y @maschwenk/tsrs -p path/to/project --noEmit   # type check only
npx -y @maschwenk/tsrs -p . --singleThreaded         # one checker thread (less memory)
```

or `pnpm add -D @maschwenk/tsrs` and run `tsrs` from scripts. [Options and defaults](#options-and-defaults) lists the
flags that matter for speed and memory.

## How fast

vscode (10,427 files) on a 64-vCPU Linux machine, each tool at its own default thread count, median of 3 runs.

| | wall time | peak memory | tsrs speedup | tsrs memory efficiency |
| --- | ---: | ---: | ---: | ---: |
| tsc 7 (tsgo 7.0.2) | 9.60 s | 7.54 GiB | **20.25x** | **2.85x** |
| `bun check` (Bun 1.4.3 canary) | 0.83 s | 2.87 GiB | 1.76x | 1.08x |
| **tsrs** | **0.47 s** | 2.65 GiB | | |

- **Against tsc 7:** 20.25x faster and 2.85x more memory efficient (2.65 GiB against 7.54 GiB).
- **Against `bun check`:** 1.76x faster and 1.08x more memory efficient on vscode. Thread for thread, tsrs is 1.83x to
  1.98x faster and `bun check` is 1.07x to 1.25x more memory efficient; across the seventeen projects tsrs is more
  memory efficient on eleven and less on five ([details](#tsrs-against-bun-check-thread-for-thread)).
- **Across the seventeen projects:** 3.90x (Compiler) to 20.25x (vscode) faster than tsgo, and faster than
  `bun check` on all seventeen, 1.47x (xstate-main) to 3.04x (nuxt) on sixteen (the mui-docs row has a caveat, in
  [More numbers](#more-numbers)).

Source: [`bench/results/2026-10-07-6db7beec77d9.md`](bench/results/2026-10-07-6db7beec77d9.md), the table below.
This code is in release 0.7.0 and later (0.6.0 and earlier predate it; 0.8.0 adds two instruction cuts that leave it within noise); to reproduce the numbers from source, build
`main` with the PGO `dist` profile ([Build from source](#build-from-source)).

The table below is generated: the bench workflow rewrites it on every push to `main`, and `bench/results/` keeps every
table it has produced.

<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2 vs bun check

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), or from a set of large open-source applications (cal-diy, formbricks-web, supabase-studio, t3code-server, mikro-orm, next-packages-next, next-root, storybook, nuxt, playwright, drizzle-orm), with tsgo 7.0.2 (npm `typescript@7.0.2`), with tsrs at commit `bec33207eb58` (the PGO-optimized `dist` build, built like the npm release binaries) and with `bun check` from Bun 1.4.3-canary.1+bd599f5af (one thread per core unless a `--threads` flag is named): `tsc -p <project> --noEmit`, median of 3 interleaved runs, on Depot CI `depot-ubuntu-24.04-64` (64 vCPU, 252 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor).

**Default mode on a 64-vCPU machine: no thread flag; tsgo uses 4 checker threads, tsrs half the cores, at least min(cores, 8), at most 32 (32 here; tsrs also resolves members lazily, its default), bun all 64 cores**

| project | errors, tsgo / tsrs / bun | tsgo wall (s) | tsrs wall (s) | bun check wall (s) | speedup vs tsgo | speedup vs bun | tsgo peak memory | tsrs peak memory | bun check peak memory | memory efficiency vs tsgo | memory efficiency vs bun |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) / 359 | 9.59 | 0.50 | 0.88 | **19.02x** | 1.74x | 7.56 GiB | 2.64 GiB | 2.85 GiB | 2.86x | 1.08x |
| next-packages-next | 6 / 5 (ref 5) / 6 | 2.74 | 0.15 | 0.42 | **18.13x** | 2.79x | 1.76 GiB | 828 MiB | 1.07 GiB | 2.18x | 1.33x |
| mui-docs | 0 / 0 / 23 | 11.55 | 0.75 | 54.49 | **15.32x** | 72.27x | 7.56 GiB | 2.09 GiB | 17.74 GiB | 3.61x | 8.47x |
| mikro-orm | 43651 / 43651 / 43651 | 9.47 | 0.66 | 1.28 | **14.41x** | 1.95x | 6.77 GiB | 2.65 GiB | 2.80 GiB | 2.56x | 1.06x |
| supabase-studio | 0 / 9 (ref 9) / 0 | 6.17 | 0.49 | 1.19 | **12.56x** | 2.43x | 4.51 GiB | 2.28 GiB | 2.12 GiB | 1.98x | 0.93x |
| storybook | 93 / 93 / 93 | 1.56 | 0.12 | 0.26 | **12.48x** | 2.08x | 1.39 GiB | 731 MiB | 859 MiB | 1.95x | 1.18x |
| webpack | 848 / 840 (ref 840) / 836 | 1.26 | 0.11 | 0.24 | **11.78x** | 2.22x | 1.12 GiB | 770 MiB | 777 MiB | 1.49x | 1.01x |
| drizzle-orm | 10845 / 10846 (ref 10846) / 10845 | 1.64 | 0.14 | 0.43 | **11.30x** | 2.94x | 1.70 GiB | 1.14 GiB | 1023 MiB | 1.49x | 0.88x |
| formbricks-web | 0 / 0 / 0 | 6.09 | 0.55 | 0.97 | **11.14x** | 1.77x | 5.54 GiB | 2.86 GiB | 2.19 GiB | 1.94x | 0.77x |
| playwright | 13 / 13 / 13 | 1.06 | 0.10 | 0.21 | **10.31x** | 2.09x | 1.09 GiB | 583 MiB | 711 MiB | 1.92x | 1.22x |
| t3code-server | 5 / 6 (ref 6) / 5 | 12.56 | 1.23 | 2.46 | **10.25x** | 2.01x | 6.35 GiB | 2.80 GiB | 1.54 GiB | 2.26x | 0.55x |
| next-root | 441 / 438 (ref 438) / 441 | 1.45 | 0.14 | 0.28 | **10.22x** | 1.97x | 1.29 GiB | 662 MiB | 864 MiB | 1.99x | 1.30x |
| cal-diy | 136 / 136 / 136 | 5.02 | 0.52 | 1.36 | **9.62x** | 2.61x | 4.56 GiB | 2.60 GiB | 1.99 GiB | 1.76x | 0.77x |
| nuxt | 3 / 4 (ref 4) / 3 | 1.21 | 0.14 | 0.42 | **8.83x** | 3.05x | 1.16 GiB | 664 MiB | 771 MiB | 1.78x | 1.16x |
| xstate-main | 0 / 0 / 0 | 0.80 | 0.12 | 0.18 | **6.42x** | 1.44x | 736 MiB | 323 MiB | 608 MiB | 2.28x | 1.88x |
| Compiler-Unions | 41 / 41 / 41 | 0.53 | 0.14 | 0.21 | 3.93x | 1.57x | 236 MiB | 110 MiB | 315 MiB | 2.14x | 2.86x |
| Compiler | 43 / 43 / 43 | 0.28 | 0.07 | 0.11 | 3.86x | 1.56x | 211 MiB | 104 MiB | 301 MiB | 2.03x | 2.89x |

errors: the number of type errors each compiler reports on the project; tsgo's and tsrs's must be equal (a bold errors cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster; bold from 5x). peak memory: maximum resident set size. memory efficiency vs tsgo: tsgo peak memory / tsrs peak memory (2x = tsrs uses half the memory; below 1 = tsrs uses more; bold from 4x). speedup vs bun: bun check wall / tsrs wall; memory efficiency vs bun: bun check peak memory / tsrs peak memory. bun check follows TypeScript 7.0, tsrs the 7.1-dev commit it ports, so their error counts can differ where the two TypeScript versions do.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-64` (64 vCPU, 252 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-07 16:42 UTC. tsrs commit: `bec33207eb58`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
<!-- bench:end -->

## tsrs against bun check, thread for thread

`bun check` is Bun's type checker. It follows TypeScript 7.0 and reports exactly the errors tsgo 7.0.2 reports (359 on
vscode). tsrs ports a TypeScript 7.1-dev commit and reports exactly the errors that nightly reports (371 on vscode).
Each matches the TypeScript version it follows, so this is a race between two tools doing the same job on the same
program.

The defaults differ. tsgo runs 4 checker threads on any machine, tsrs half the cores (every core up to 8) up to 32 (32 here), `bun check` one
thread per core (64). The table gives every tool the same thread count: `--checkers N` for tsgo and tsrs, `--threads N`
for `bun check`. vscode, 64-vCPU machine, mean of 20 interleaved runs per cell, wall time then peak memory.

| threads | tsgo 7.0.2 | tsrs | `bun check` |
| ---: | ---: | ---: | ---: |
| 4 | 9.75 s, 7.43 GiB | 2.43 s, 1.83 GiB | 4.57 s, 1.46 GiB |
| 8 | 6.13 s, 7.99 GiB | 1.35 s, 1.97 GiB | 2.47 s, 1.60 GiB |
| 16 | 4.54 s, 8.67 GiB | 0.81 s, 2.13 GiB | 1.49 s, 1.85 GiB |
| 64 | 4.01 s, 11.58 GiB | 0.44 s, 3.06 GiB | 0.87 s, 2.86 GiB |

- tsrs is faster than `bun check` at every thread count, 1.83x to 1.98x.
- `bun check` is more memory efficient than tsrs at every thread count: 1.25x at 4 threads, 1.23x at 8, 1.15x at 16,
  1.07x at 64. At each tool's default (tsrs 32 checker threads, `bun check` 64) tsrs is 1.64x faster and 1.08x more
  memory efficient: 0.53 s and 2.64 GiB against 0.87 s and 2.86 GiB.
- `--threads` limits all of `bun check`'s threads, while `--checkers` limits only the checker threads of tsgo and tsrs
  (parsing still uses every core), so at small counts `bun check` is held back harder.

Source: [`bench/results/compare/2026-10-07-ae33f10b1b8b-64t.md`](bench/results/compare/2026-10-07-ae33f10b1b8b-64t.md);
the percentages and ratios are computed from its rows.

### More numbers

- **A 20-run mean at each tool's default** (same file as the table above): tsgo 9.79 s and 7.52 GiB, tsrs 0.53 s and
  2.64 GiB, `bun check` 0.87 s and 2.86 GiB, which is 18.42x faster than tsgo and 1.64x faster than `bun check`.
- **Against tsgo at the same thread counts:** tsrs is 4.01x to 9.17x faster (the "same threads" column of that file).
- **Errors:** `bun check`'s errors equal tsgo 7.0.2's line for line ([`bench/README.md`](bench/README.md), "Head-to-head
  on a wide machine", which also says how to rerun the head-to-head).
- **Speed across the ten projects** at each tool's default (median of 3,
  [`bench/results/2026-10-07-bd86c6d7a6ba.md`](bench/results/2026-10-07-bd86c6d7a6ba.md)): tsrs is faster than
  `bun check` on all of them, 1.29x to 2.38x on nine. The tenth is mui-docs: `bun check` took 40.53 s and 18.24 GiB and
  reported 23 errors where tsgo and tsrs report none, so do not read its 48.37x as a speed comparison.
- **Memory across the ten projects** is mixed. tsrs is more memory efficient than `bun check` on xstate-main (1.73x),
  Compiler (2.94x), Compiler-Unions (2.88x) and mui-docs (8.20x, with the caveat above), level on vscode and webpack
  (1.00x, 0.99x), and less on cal-diy, formbricks-web, supabase-studio and t3code-server, where `bun check` is 1.36x,
  1.43x, 1.18x and 1.90x more memory efficient.
- **On an 8-vCPU machine** (default settings, ten projects, median of 3 runs, same results file): 3.63x to 9.38x faster
  than tsgo 7.0.2, and 2.19x to 7.79x more memory efficient. vscode is 4.08x faster, and 2.86x faster with one checker thread in
  each (10.57 s against 30.20 s), so threads are not the whole gain.
- **Where the runs happened:** Depot CI `depot-ubuntu-24.04-64` for the 64-vCPU rows and `depot-ubuntu-24.04-8` for the
  8-vCPU ones, Linux x86_64, AMD EPYC 9R45. tsc 7 is TypeScript's Go compiler, `typescript@7.0.2` from npm. tsrs is a
  PGO `dist` build: commit `311fcea33269` in the median-of-3 table at the top, `ae33f10b1b8b` in the 20-run mean,
  `bd86c6d7a6ba` ([PR #121](https://github.com/maschwenk/tsrs/pull/121)) in the ten-project bullets above.

## What tsrs is

- **A compiler.** It is `tsc`: same flags, same tsconfig, same diagnostics, byte for byte. It emits by default like tsc
  does (JavaScript, declarations, source maps, tsbuildinfo, `-b`), unless the options turn that off (`--noEmit`,
  `emitDeclarationOnly`, `noEmitOnError`). [`docs/EMIT.md`](docs/EMIT.md) describes the emit port.
- **A language server.** It is `tsgo --lsp`: `tsrs --lsp -stdio`. See [Language server](#language-server).
- **A Node API server.** `tsrs --api` serves TypeScript 7's Node API (`unstable/sync`, `unstable/async`), with the gaps
  listed below and in [`docs/NODE_API.md`](docs/NODE_API.md).

**Go is the spec.** tsrs ports the Go implementation in [microsoft/TypeScript](https://github.com/microsoft/TypeScript)
(`tsc/internal`) at commit `b85298b6a81f` (TypeScript 7.1.0-dev.20260929) function for function. The Go code is the
specification: a difference from tsgo at that commit (diagnostics, their order, message text, exit code) is a tsrs bug,
even when tsrs's output looks more reasonable ([`CONTRIBUTING.md`](CONTRIBUTING.md)). On the TypeScript conformance
suite the output matches the reference on 13,458 of 13,462 error baselines and on all 12,779 `.types` and `.symbols`
baselines; the four exceptions are test-harness artifacts. These counts are in Go-compatible mode (`--checkerAssignment
go`). By default, tsrs makes output independent of how files are split over checkers, which changes one test's
`.types` / `.symbols` / `.d.ts` baselines by design (notes/perf-order-independence.md).
[`docs/STATUS.md`](docs/STATUS.md) has the details and the comparison on the 38k-file codebase.

## What it does and doesn't do

Starting with `0.3.0`, the npm package includes default emit, incremental builds and the `unstable/*` Node API.
Use `--noEmit` for type checking only; `TSRS_EMIT` no longer changes behavior. "Match" and "identical" below are
against tsgo built from the same pinned commit.

| feature | status | evidence and notes |
| --- | --- | --- |
| Type checking (`tsc --noEmit`) | yes | 13,458 of 13,462 error baselines and all 12,779 `.types` / `.symbols` baselines match; same diagnostics on the 38k-file codebase |
| Checker threads, `--singleThreaded`, `--pretty`, `--extendedDiagnostics`, `--listFiles`, `--listFilesOnly` | yes | checkers steal unstarted files from the busiest checker: -11 to -17% wall at 4-8 checkers on the 38k-file codebase (notes/perf-checker-stealing.md). Default checker count since `0.5.0`: half the cores, 4 to 32 (was 4 to 8): vscode on a 64-vCPU machine 1.73 s -> 0.82 s for +0.9 GiB peak (notes/perf-checker-64.md) |
| Output independent of the checker count and assignment | yes (tsgo: no) | diagnostics and `.d.ts` identical for `--checkers 1`-16 and 20 random assignments on the error-rich corpora; `--checkerAssignment go` keeps tsgo's history-dependent output for byte-identity (notes/perf-order-independence.md) |
| Options TypeScript 7 removed (ES5 target, AMD/UMD/System modules, `node10`/`classic` resolution, `baseUrl`, …) | rejected, as in tsgo | same TS5102 / TS5108 errors |
| JavaScript emit | yes (since `0.3.0`) | all of tsgo's script transforms: type erasure, import elision, enums, namespaces, const enum inlining, CommonJS and ES modules, JSX, legacy and standard decorators, class fields, `using`, async and `for await`, and lowering for older targets (object rest/spread, `?.`, `??`, `**`, logical assignment). 13,392 `.js` baselines pass, 0 fail |
| Declaration emit (`.d.ts`) | yes (since `0.3.0`) | part of the `.js` baselines |
| Source maps, declaration maps | yes (since `0.3.0`) | 149 `.js.map` and 156 `.sourcemap.txt` baselines pass, 0 fail |
| Incremental compilation (`.tsbuildinfo`) | yes (since `0.3.0`) | tsbuildinfo files identical, also for `--noEmit` programs |
| `--build` (project references, `--builders`, `--clean`, `--dry`, `--force`, `--verbose`) | yes (since `0.3.0`) | 187 of 190 `tsbuild` and 187 of 216 `tsc` scenario baselines pass; the failures are CLI outputs tsrs does not port (`--help`, `--init`, `--showConfig`, `--locale`, `--generateTrace`, the `--version` line) and one message that prints an internal symbol name. Finished projects stay in memory (Go frees them), so peak memory grows with the graph |
| Emit on the 38k-file codebase | yes (since `0.3.0`) | all 96 packages built with `tsc`: 9,257 output files (JS, `.d.ts`, maps, `.tsbuildinfo`) byte-identical, same diagnostics and exit codes |
| `--watch` (also `-b --watch`) | no | exits with "not supported" |
| `--init`, `--showConfig` | no | exits with "not supported"; `--help` prints a short usage, not tsc's |
| `--locale` | ignored | messages are English only |
| `--generateTrace` | ignored | no trace is written |
| Language server (`--lsp -stdio`) | yes | 4,066 of 4,546 fourslash tests pass; 13,869 of 13,869 responses identical in an xstate editor session (below) |
| Content mappers, automatic type acquisition, telemetry, pprof requests (language server) | no | not ported |
| `--api` (the IPC server behind TypeScript 7's Node API: `unstable/sync` MessagePack, `unstable/async` JSON-RPC) | yes, with gaps | pinned upstream client suites against a release build of the integration branch: `test/sync/api.test.ts` 339/339, `test/async/api.test.ts` 348/348, `ast` 111/111, `astnav` 4/4 + 4/4, `api-generators` 43/43; suite passes are not byte-level response parity. Not implemented: CPU/heap profiling requests; `getCurrentLanguageServerSnapshot` returns the standalone-session error (no LSP-attached API session); no Windows named pipes. Per-method status in `docs/NODE_API.md` |
| Prebuilt binaries | macOS arm64, Linux x64/arm64 (glibc) | no Windows or Intel macOS binary |

## Options and defaults

`--extendedDiagnostics` prints the usual counters.
Without `--checkers`, tsrs runs one checker thread per core up to 8 and half as many as the machine has cores above
that, at least tsgo's 4 and at most 32 (8 on 8 cores and on 16, 9 on 18, 32 on 64; 4 for small programs and in `-b`
build mode; up to 0.4.0 the cap was 8, and up to 0.6.0 an 8-core machine got 4); diagnostics do not depend on the
count, the `--extendedDiagnostics` counters and peak memory do.
In a `--noEmit` check, tsrs frees the syntax tree and binder output of a test, spec, story or mock file once it is
checked, when the program gets at most 16 checkers (vscode: -13 to -16% peak memory; `TSRS_FREE_LEAVES=0` turns it
off; notes/mem-free-leaf-files.md).
`--checkerCostCache <file>` (opt-in) records per-file check times in `<file>` and balances the checker threads on
them in the next run (a few percent to ~15% less wall time on repeated runs; it never changes diagnostics).
By default tsrs also runs with checker changes that are not merged upstream yet; `--noLazyMembers` turns them
off and gives the reference-identical mode. None of them changes any diagnostic (verified on the whole conformance
suite, errors, types and symbols):

Open upstream pull request:

- https://github.com/microsoft/TypeScript/pull/64475

Go patches prepared from this port (`upstream/`), not opened:

* [give tuple references lazy member tables](upstream/pr-01-tuple-lazy-tables.md)
* [answer empty-object checks from lazy member tables](upstream/pr-02-empty-object-lazy-tables.md)
* [find unmatched properties without instantiating a lazy target's members](upstream/pr-03-unmatched-properties-lazy-target.md)

Opened upstream and closed as too small on their own (keyof mapped types, and the patches for the
[union property cache](upstream/pr-04-union-property-cache.md) and the
[conditional instantiation mapper](upstream/pr-05-conditional-instantiation-mapper.md)):

- https://github.com/microsoft/TypeScript/pull/64526
- https://github.com/microsoft/TypeScript/pull/64600
- https://github.com/microsoft/TypeScript/pull/64601

## Language server

`tsrs --lsp -stdio` is a port of the TypeScript 7 language server (Go's `internal/lsp`, `internal/ls`,
`internal/project`): diagnostics, hover, definitions, references, rename, completions with auto-imports, signature
help, symbols, semantic tokens, folding, inlay hints, call hierarchy, code fixes, organize imports, formatting,
file watching and cancellation.

* TypeScript's fourslash tests: 4,066 of 4,546 pass; the 63 failures need content mappers (not ported) or `tsc -b`
  with emit, and the other 417 are skipped in the Go implementation too.
* Replaying the same editor session against the Go server and comparing the JSON responses: 13,869 of 13,869
  identical on xstate (diagnostics, hover, definition, references, completion, signature help, symbols, with edits).
* Memory on the 38k-file codebase over a 200-edit session: 2.9 GiB and flat (0.01 MiB per edit); the Go server goes
  from 4.9 to 7.4 GiB in the same session.

Neovim 0.11+:

```lua
vim.lsp.config('tsrs', {
  cmd = { 'tsrs', '--lsp', '-stdio' },
  filetypes = { 'typescript', 'typescriptreact', 'javascript', 'javascriptreact' },
  root_markers = { 'tsconfig.json', 'jsconfig.json', 'package.json', '.git' },
})
vim.lsp.enable('tsrs')
```

VS Code: the TypeScript 7 extension starts whatever executable named `tsgo` it finds in `js/ts.tsdk.path`, so point
that setting at a directory holding a `tsgo` symlink to the tsrs binary. [`docs/LSP.md`](docs/LSP.md) has both setups in full, the
design, the test tables and the known gaps.

## Build from source

```sh
cargo build --release -p tsrs_cli -p tsrs_testrunner
./target/release/tsrs -p path/to/project
./target/release/tsrs-test run --suite all --baselines types,symbols   # conformance suite, ~20 s
cargo build --release -p tsrs_fourslash && ./target/release/tsrs-fourslash run   # language-service tests, ~5 s
```

`CONTRIBUTING.md` covers the workflow; `docs/PORTING.md`, `docs/AST.md`, `docs/CHECKER.md` and `docs/LSP.md` the conventions;
`tools/oracle/` the Go oracle programs that compare each stage against the reference.

The benchmark numbers above use the PGO `dist` build the release workflow makes, not `cargo build --release`
(notes/perf-pgo.md measured PGO at 7-14% less CPU time). [`bench/README.md`](bench/README.md) says how each number is
measured and how to rerun it.

## Docs

- [`docs/STATUS.md`](docs/STATUS.md): the status log, release notes, and the comparison on the 38k-file codebase.
- [`docs/EMIT.md`](docs/EMIT.md): the JavaScript, declaration and source map emit port.
- [`docs/LSP.md`](docs/LSP.md): the language server, editor setups, test tables and known gaps.
- [`docs/NODE_API.md`](docs/NODE_API.md): the Node API, method by method.
- [`docs/DEBUGGING.md`](docs/DEBUGGING.md): how to find and fix a difference from tsgo, and the Go oracle programs.
- [`bench/README.md`](bench/README.md): how the benchmarks are measured, and how to rerun them.
- `notes/`: one note per performance change or experiment, with the numbers.

## Contributing

Bug reports with a small reproduction are the most useful contribution; fixes are welcome too. The fidelity rule above
decides what counts as a bug. [`CONTRIBUTING.md`](CONTRIBUTING.md) has the workflow, [`SECURITY.md`](SECURITY.md) says
how to report a vulnerability privately, and everyone follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## Provenance

Derivative work of microsoft/TypeScript (Copyright Microsoft Corporation), Apache-2.0 like the original
([`LICENSE`](LICENSE), [`NOTICE`](NOTICE)); the bundled `lib.*.d.ts` files are TypeScript's. Not affiliated with or endorsed by
Microsoft. The port was written largely by AI coding agents directed and reviewed by Max Schwenk; its correctness
rests on the evidence above rather than on review alone.

# tsrs

[![npm](https://img.shields.io/npm/v/@maschwenk/tsrs)](https://www.npmjs.com/package/@maschwenk/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

**tsrs is a Rust port of the TypeScript 7 compiler and language server.** It takes `tsc`'s flags and tsconfig and
reports the same diagnostics as the Go compiler it ports. On the vscode codebase, on a 64-vCPU machine, it type-checks
[about 16x faster than tsc 7 and 1.41x faster than `bun check`](#benchmark-tsrs-vs-tsgo-702-vs-bun-check).
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

| | wall time | peak memory | tsrs speedup | tsrs memory / its |
| --- | ---: | ---: | ---: | ---: |
| tsc 7 (tsgo 7.0.2) | 9.65 s | 7.56 GiB | **16.11x** | 0.38x |
| `bun check` (Bun 1.4.3 canary) | 0.84 s | 2.87 GiB | 1.41x | 1.00x |
| **tsrs** | **0.60 s** | 2.87 GiB | | |

- **Against tsc 7:** 16.11x faster, at 0.38x of its memory (2.87 GiB against 7.56 GiB).
- **Against `bun check`:** 1.41x faster at the same peak memory on vscode. Thread for thread, `bun check` uses 14% to
  35% less memory, and across the ten projects tsrs uses more on four, the same on two and less on four
  ([details](#tsrs-against-bun-check-thread-for-thread)).
- **Across the ten projects:** 3.43x (Compiler) to 16.11x (vscode) faster than tsgo, and faster than `bun check` on all
  ten (the mui-docs row has a caveat, in [More numbers](#more-numbers)).

Source: [`bench/results/2026-10-07-bd86c6d7a6ba.md`](bench/results/2026-10-07-bd86c6d7a6ba.md), the table below.
This code is in release 0.5.0 and later (0.4.0 and earlier predate it); to reproduce the numbers from source, build
`main` with the PGO `dist` profile ([Build from source](#build-from-source)).

The table below is generated: the bench workflow rewrites it on every push to `main`, and `bench/results/` keeps every
table it has produced.

<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2 vs bun check

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), or from a set of large open-source applications (cal-diy, formbricks-web, supabase-studio, t3code-server, mikro-orm, next-packages-next, next-root, storybook, nuxt, playwright, drizzle-orm), with tsgo 7.0.2 (npm `typescript@7.0.2`), with tsrs at commit `afb54cb51434` (the PGO-optimized `dist` build, built like the npm release binaries) and with `bun check` from Bun 1.4.3-canary.1+bd599f5af (one thread per core unless a `--threads` flag is named): `tsc -p <project> --noEmit`, median of 3 interleaved runs, on Depot CI `depot-ubuntu-24.04-64` (64 vCPU, 252 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor).

**Default mode on a 64-vCPU machine: no thread flag; tsgo uses 4 checker threads, tsrs half the cores clamped to 4..32 (32 here; tsrs also resolves members lazily, its default), bun all 64 cores**

| project | errors, tsgo / tsrs / bun | tsgo wall (s) | tsrs wall (s) | bun check wall (s) | speedup vs tsgo | speedup vs bun | tsgo peak memory | tsrs peak memory | bun check peak memory | memory, tsrs / tsgo | memory, tsrs / bun |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) / 359 | 9.57 | 0.54 | 0.84 | **17.72x** | 1.56x | 7.69 GiB | 2.74 GiB | 2.87 GiB | 0.36x | 0.96x |
| xstate-main | 0 / 0 / 0 | 0.79 | 0.13 | 0.18 | **6.18x** | 1.39x | 737 MiB | 327 MiB | 611 MiB | 0.44x | 0.53x |
| webpack | 848 / 840 (ref 840) / 836 | 1.25 | 0.12 | 0.24 | **10.90x** | 2.06x | 1.16 GiB | 744 MiB | 773 MiB | 0.63x | 0.96x |
| mui-docs | 0 / 0 / 23 | 11.59 | 0.79 | 42.60 | **14.67x** | 53.92x | 7.56 GiB | 2.17 GiB | 17.70 GiB | 0.29x | 0.12x |
| Compiler | 43 / 43 / 43 | 0.28 | 0.07 | 0.12 | 3.84x | 1.58x | 209 MiB | 106 MiB | 308 MiB | 0.51x | 0.34x |
| Compiler-Unions | 41 / 41 / 41 | 0.53 | 0.13 | 0.21 | 4.14x | 1.63x | 237 MiB | 110 MiB | 309 MiB | 0.46x | 0.36x |
| cal-diy | 136 / 136 / 136 | 4.97 | 0.52 | 1.33 | **9.50x** | 2.54x | 4.57 GiB | 2.63 GiB | 2.03 GiB | 0.58x | 1.30x |
| formbricks-web | 0 / 0 / 0 | 6.22 | 0.61 | 0.94 | **10.14x** | 1.54x | 5.53 GiB | 2.92 GiB | 2.21 GiB | 0.53x | 1.32x |
| supabase-studio | 0 / 9 (ref 9) / 0 | 6.14 | 0.53 | 1.19 | **11.57x** | 2.24x | 4.54 GiB | 2.36 GiB | 2.13 GiB | 0.52x | 1.11x |
| t3code-server | 5 / 6 (ref 6) / 5 | 12.33 | 1.34 | 2.47 | **9.21x** | 1.84x | 6.22 GiB | 2.84 GiB | 1.55 GiB | 0.46x | 1.83x |
| mikro-orm | 43651 / 43651 / 43651 | 9.45 | 0.64 | 1.26 | **14.88x** | 1.98x | 6.68 GiB | 2.84 GiB | 2.86 GiB | 0.42x | 0.99x |
| next-packages-next | 6 / 5 (ref 5) / 6 | 2.72 | 0.15 | 0.41 | **18.25x** | 2.74x | 1.80 GiB | 851 MiB | 1.08 GiB | 0.46x | 0.77x |
| next-root | 441 / 438 (ref 438) / 441 | 1.43 | 0.14 | 0.28 | **9.97x** | 1.94x | 1.28 GiB | 680 MiB | 861 MiB | 0.52x | 0.79x |
| storybook | 93 / 93 / 93 | 1.51 | 0.13 | 0.26 | **11.93x** | 2.06x | 1.42 GiB | 722 MiB | 864 MiB | 0.50x | 0.84x |
| nuxt | 3 / 4 (ref 4) / 3 | 1.25 | 0.15 | 0.43 | **8.23x** | 2.80x | 1.17 GiB | 678 MiB | 774 MiB | 0.57x | 0.88x |
| playwright | 13 / 13 / 13 | 1.07 | 0.11 | 0.21 | **9.59x** | 1.92x | 1.05 GiB | 612 MiB | 710 MiB | 0.57x | 0.86x |
| drizzle-orm | **10845 / 10845 (locations differ)** / 10845 | 1.63 | 0.16 | 0.42 | **10.45x** | 2.69x | 1.71 GiB | 1.08 GiB | 1.01 GiB | 0.63x | 1.08x |

errors: the number of type errors each compiler reports on the project; tsgo's and tsrs's must be equal (a bold errors cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster; bold from 5x). peak memory: maximum resident set size. memory, tsrs / tsgo: below 1 = tsrs uses less (bold at 0.25x or less, a quarter of tsgo's memory). speedup vs bun: bun check wall / tsrs wall; memory, tsrs / bun: below 1 = tsrs uses less. bun check follows TypeScript 7.0, tsrs the 7.1-dev commit it ports, so their error counts can differ where the two TypeScript versions do.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-64` (64 vCPU, 252 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-07 09:03 UTC. tsrs commit: `afb54cb51434`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
<!-- bench:end -->

## tsrs against bun check, thread for thread

`bun check` is Bun's type checker. It follows TypeScript 7.0 and reports exactly the errors tsgo 7.0.2 reports (359 on
vscode). tsrs ports a TypeScript 7.1-dev commit and reports exactly the errors that nightly reports (371 on vscode).
Each matches the TypeScript version it follows, so this is a race between two tools doing the same job on the same
program.

The defaults differ. tsgo runs 4 checker threads on any machine, tsrs half the cores up to 32 (32 here), `bun check` one
thread per core (64). The table gives every tool the same thread count: `--checkers N` for tsgo and tsrs, `--threads N`
for `bun check`. vscode, 64-vCPU machine, mean of 20 interleaved runs per cell, wall time then peak memory.

| threads | tsgo 7.0.2 | tsrs | `bun check` |
| ---: | ---: | ---: | ---: |
| 4 | 9.63 s, 7.48 GiB | 2.49 s, 2.23 GiB | 4.40 s, 1.45 GiB |
| 8 | 6.08 s, 7.93 GiB | 1.39 s, 2.37 GiB | 2.40 s, 1.59 GiB |
| 16 | 4.54 s, 8.67 GiB | 0.86 s, 2.57 GiB | 1.45 s, 1.84 GiB |
| 64 | 3.95 s, 11.63 GiB | 0.59 s, 3.31 GiB | 0.87 s, 2.85 GiB |

- tsrs is faster than `bun check` at every thread count, 1.47x to 1.77x.
- `bun check` uses less memory than tsrs at every thread count: 35% less at 4 threads, 33% at 8, 28% at 16, 14% at 64.
  At each tool's default the two peak at the same memory (tsrs runs 32 checker threads there, `bun check` 64).
- `--threads` limits all of `bun check`'s threads, while `--checkers` limits only the checker threads of tsgo and tsrs
  (parsing still uses every core), so at small counts `bun check` is held back harder.

Source: [`bench/results/compare/2026-10-07-b05f05bc6e3f-64t.md`](bench/results/compare/2026-10-07-b05f05bc6e3f-64t.md);
the percentages and ratios are computed from its rows.

### More numbers

- **A 20-run mean at each tool's default** (same file as the table above): tsgo 9.62 s and 7.50 GiB, tsrs 0.60 s and
  2.87 GiB, `bun check` 0.87 s and 2.85 GiB, which is 15.96x faster than tsgo and 1.45x faster than `bun check`.
- **Against tsgo at the same thread counts:** tsrs is 3.87x to 6.67x faster (the "same threads" column of that file).
- **Errors:** `bun check`'s errors equal tsgo 7.0.2's line for line ([`bench/README.md`](bench/README.md), "Head-to-head
  on a wide machine", which also says how to rerun the head-to-head).
- **Speed across the ten projects** at each tool's default (median of 3,
  [`bench/results/2026-10-07-bd86c6d7a6ba.md`](bench/results/2026-10-07-bd86c6d7a6ba.md)): tsrs is faster than
  `bun check` on all of them, 1.29x to 2.38x on nine. The tenth is mui-docs: `bun check` took 40.53 s and 18.24 GiB and
  reported 23 errors where tsgo and tsrs report none, so do not read its 48.37x as a speed comparison.
- **Memory across the ten projects** is mixed. tsrs uses less than `bun check` on xstate-main (0.58x), Compiler (0.34x),
  Compiler-Unions (0.35x) and mui-docs (0.12x, with the caveat above), about the same on vscode and webpack (1.00x, 1.01x), and
  more on cal-diy (1.36x), formbricks-web (1.43x), supabase-studio (1.18x) and t3code-server (1.90x).
- **On an 8-vCPU machine** (default settings, ten projects, median of 3 runs, same results file): 3.63x to 9.38x faster
  than tsgo 7.0.2, at 0.13x to 0.46x of its memory. vscode is 4.08x faster, and 2.86x faster with one checker thread in
  each (10.57 s against 30.20 s), so threads are not the whole gain.
- **Where the runs happened:** Depot CI `depot-ubuntu-24.04-64` for the 64-vCPU rows and `depot-ubuntu-24.04-8` for the
  8-vCPU ones, Linux x86_64, AMD EPYC 9R45. tsc 7 is TypeScript's Go compiler, `typescript@7.0.2` from npm. tsrs is a
  PGO `dist` build of [PR #121](https://github.com/maschwenk/tsrs/pull/121): commit `bd86c6d7a6ba` in the median-of-3
  table, `b05f05bc6e3f` in the 20-run mean, with the same source in `crates/`.

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
Without `--checkers`, tsrs runs half as many checker threads as the machine has cores, at least tsgo's 4 and at most
32 (4 on 8 cores, 32 on 64; 4 for small programs and in `-b` build mode; up to 0.4.0 the cap was 8); diagnostics do not
depend on the count, the `--extendedDiagnostics` counters and peak memory do.
`--checkerCostCache <file>` (opt-in) records per-file check times in `<file>` and balances the checker threads on
them in the next run (a few percent to ~15% less wall time on repeated runs; it never changes diagnostics).
By default tsrs also runs with checker changes that are not merged upstream yet; `--noLazyMembers` turns them
off and gives the reference-identical mode. None of them changes any diagnostic (verified on the whole conformance
suite, errors, types and symbols):

* [build member tables of instantiated classes/interfaces lazily (microsoft/TypeScript#64475)](https://github.com/microsoft/TypeScript/pull/64475)
* [build members of keyof mapped types lazily (microsoft/TypeScript#64526)](https://github.com/microsoft/TypeScript/pull/64526)
* [give tuple references lazy member tables](upstream/pr-01-tuple-lazy-tables.md)
* [answer empty-object checks from lazy member tables](upstream/pr-02-empty-object-lazy-tables.md)
* [find unmatched properties without instantiating a lazy target's members](upstream/pr-03-unmatched-properties-lazy-target.md)
* [don't copy union/intersection properties into the augmented property cache](upstream/pr-04-union-property-cache.md)
* [instantiate conditional types without a combined mapper for the cache lookup](upstream/pr-05-conditional-instantiation-mapper.md)

The last five are Go patches prepared from this port (`upstream/`); the last two were opened and withdrawn as too small on their own (microsoft/TypeScript#64600, #64601).

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

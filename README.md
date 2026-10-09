# tsrs

[![npm](https://img.shields.io/npm/v/@maschwenk/tsrs)](https://www.npmjs.com/package/@maschwenk/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

**tsrs is a Rust port of the TypeScript 7 compiler and language server.** It takes `tsc`'s flags and tsconfig and
reports the same diagnostics as the Go compiler it ports. On the vscode codebase, on a 64-vCPU machine, it type-checks
[17.7x faster and 2.8x more memory efficient than tsc 7, and 1.55x faster than `bun check`](#benchmark-tsrs-vs-tsgo-702-vs-bun-check).
On the TypeScript conformance suite it matches the reference on 13,458 of 13,462 error baselines and on all 12,779
`.types` and `.symbols` baselines, in Go-compatible mode ([evidence](#what-tsrs-is)).

[Quick start](#quick-start) · [How fast](#how-fast) · [vs bun check](#tsrs-against-bun-check-thread-for-thread) ·
[What it does and doesn't do](#what-it-does-and-doesnt-do) · [Language server](#language-server) · [WebAssembly](#webassembly) · [Docs](#docs)

## Quick start

```sh
npx -y @maschwenk/tsrs -p path/to/project            # macOS arm64, Linux x64/arm64; emits like tsc
npx -y @maschwenk/tsrs -p path/to/project --noEmit   # type check only
npx -y @maschwenk/tsrs -p . --singleThreaded         # one checker thread (less memory)
```

or `pnpm add -D @maschwenk/tsrs` and run `tsrs` from scripts. [Options and defaults](#options-and-defaults) lists the
flags that matter for speed and memory.

## How fast

vscode (10,427 files) on a 64-vCPU Linux machine, each tool at its own default thread count, mean of 20 interleaved
runs ([`bench/results/compare/2026-10-07-ae33f10b1b8b-64t.md`](bench/results/compare/2026-10-07-ae33f10b1b8b-64t.md)).

| | wall time | peak memory | tsrs speedup | tsrs memory efficiency |
| --- | ---: | ---: | ---: | ---: |
| tsc 7 (tsgo 7.0.2) | 9.79 s | 7.52 GiB | **18.47x** | **2.85x** |
| `bun check` (Bun 1.4.3 canary) | 0.87 s | 2.86 GiB | 1.64x | 1.08x |
| **tsrs** | **0.53 s** | 2.64 GiB | | |

- **Against tsc 7:** 18.47x faster and 2.85x more memory efficient (2.64 GiB against 7.52 GiB).
- **Against `bun check`:** 1.64x faster and 1.08x more memory efficient on vscode. Thread for thread, tsrs is 1.83x to
  1.98x faster and `bun check` is 1.07x to 1.25x more memory efficient; across the seventeen projects tsrs is more
  memory efficient on eleven and less on five ([details](#tsrs-against-bun-check-thread-for-thread)).
- **Across the seventeen projects:** 3.90x (Compiler) to 20.25x (vscode) faster than tsgo, and faster than
  `bun check` on all seventeen, 1.47x (xstate-main) to 3.04x (nuxt) on sixteen (the mui-docs row has a caveat, in
  [More numbers](#more-numbers)).
- **In WebAssembly, against ts-rust's wasm build:** 2.7x (Compiler-Unions) to 7.0x (nuxt) faster and 1.2x to 2.6x less
  memory on nine projects, both single-threaded ([details](#against-ts-rusts-webassembly-build); measured by hand on a
  Mac, not in CI).

Source for the per-project figures: [`bench/results/2026-10-07-6db7beec77d9.md`](bench/results/2026-10-07-6db7beec77d9.md),
the table below, whose single-publish medians move by a few percent between runs of the same code (the 20-run mean
above is the steadier headline; the single-threaded instruction counts in `bench/results/` are the deterministic measure).
This code is in release 0.7.0 and later (0.6.0 and earlier predate it; 0.8.0 adds two instruction cuts that leave it within noise; 0.9.0 uses 2-12% less peak memory at the default checker count at the same speed); to reproduce the numbers from source, build
`main` with the PGO `dist` profile ([Build from source](#build-from-source)).

The table below is generated: the bench workflow rewrites it on every push to `main`, and `bench/results/` keeps every
table it has produced.

<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2 vs bun check

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), or from a set of large open-source applications (cal-diy, formbricks-web, supabase-studio, t3code-server, mikro-orm, next-packages-next, next-root, storybook, nuxt, playwright, drizzle-orm), with tsgo 7.0.2 (npm `typescript@7.0.2`), with tsrs at commit `2fc4b5051938` (the PGO-optimized `dist` build, built like the npm release binaries) and with `bun check` from Bun 1.4.3-canary.1+e655c5803 (one thread per core unless a `--threads` flag is named): `tsc -p <project> --noEmit`, median of 10 interleaved runs (3 for tsgo), on Depot CI `depot-ubuntu-24.04-16` (16 vCPU, 63 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Walls on this shared machine move by a few percent between runs of the same code; the single-threaded instruction counts are the deterministic measure.

**Default mode on a 16-vCPU machine: no thread flag; tsgo uses 4 checker threads, tsrs half the cores, at least min(cores, 8), at most 32 (8 here; tsrs also resolves members lazily, its default), bun all 16 cores**

| project | errors, tsgo / tsrs / bun | tsgo wall (s) | tsrs wall (s) | bun check wall (s) | speedup vs tsgo | speedup vs bun | tsgo peak memory | tsrs peak memory | bun check peak memory | memory efficiency vs tsgo | memory efficiency vs bun |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| mui-docs | 0 / 0 / 0 | 11.84 | 1.07 | 7.27 | **11.04x** | 6.77x | 7.37 GiB | 1.29 GiB | 11.69 GiB | **5.70x** | 9.05x |
| next-packages-next | 6 / 5 (ref 5) / 6 | 2.87 | 0.31 | 0.38 | **9.33x** | 1.23x | 1.76 GiB | 614 MiB | 742 MiB | 2.93x | 1.21x |
| mikro-orm | 43651 / 43651 / 43651 | 10.06 | 1.21 | 1.87 | **8.29x** | 1.54x | 6.30 GiB | 1.60 GiB | 1.40 GiB | 3.94x | 0.87x |
| drizzle-orm | 10845 / 10846 (ref 10846) / 10845 | 1.77 | 0.22 | 0.41 | **8.12x** | 1.86x | 1.70 GiB | 593 MiB | 734 MiB | 2.94x | 1.24x |
| formbricks-web | 0 / 0 / 0 | 7.12 | 0.89 | 1.07 | **8.00x** | 1.21x | 5.85 GiB | 1.67 GiB | 1.61 GiB | 3.50x | 0.96x |
| vscode | 359 / 371 (ref 371) / 359 | 10.60 | 1.37 | 1.53 | **7.73x** | 1.12x | 7.49 GiB | 1.93 GiB | 1.87 GiB | 3.89x | 0.97x |
| t3code-server | 5 / 6 (ref 6) / 5 | 13.13 | 1.72 | 2.74 | **7.64x** | 1.59x | 6.17 GiB | 1.77 GiB | 1.12 GiB | 3.49x | 0.63x |
| storybook | 93 / 93 / 93 | 1.60 | 0.22 | 0.28 | **7.25x** | 1.25x | 1.44 GiB | 457 MiB | 607 MiB | 3.24x | 1.33x |
| supabase-studio | 0 / 9 (ref 9) / 0 | 6.67 | 0.94 | 1.60 | **7.13x** | 1.71x | 4.59 GiB | 1.38 GiB | 1.22 GiB | 3.32x | 0.88x |
| webpack | 848 / 840 (ref 840) / 837 | 1.27 | 0.19 | 0.27 | **6.82x** | 1.45x | 1.15 GiB | 453 MiB | 497 MiB | 2.61x | 1.10x |
| next-root | 441 / 438 (ref 438) / 441 | 1.50 | 0.22 | 0.29 | **6.82x** | 1.32x | 1.29 GiB | 435 MiB | 568 MiB | 3.03x | 1.31x |
| playwright | 13 / 13 / 13 | 1.10 | 0.17 | 0.22 | **6.49x** | 1.30x | 1.08 GiB | 410 MiB | 477 MiB | 2.70x | 1.16x |
| cal-diy | 136 / 136 / 136 | 5.59 | 0.88 | 1.53 | **6.32x** | 1.73x | 4.33 GiB | 1.44 GiB | 1.31 GiB | 3.01x | 0.91x |
| xstate-main | 0 / 0 / 0 | 0.78 | 0.13 | 0.15 | **6.17x** | 1.21x | 742 MiB | 279 MiB | 394 MiB | 2.66x | 1.41x |
| nuxt | 3 / 4 (ref 4) / 3 | 1.31 | 0.21 | 0.44 | **6.16x** | 2.06x | 1.14 GiB | 442 MiB | 544 MiB | 2.65x | 1.23x |
| Compiler-Unions | 41 / 41 / 41 | 0.51 | 0.13 | 0.20 | 3.98x | 1.57x | 239 MiB | 100 MiB | 234 MiB | 2.40x | 2.34x |
| Compiler | 43 / 43 / 43 | 0.27 | 0.07 | 0.11 | 3.87x | 1.60x | 217 MiB | 98 MiB | 224 MiB | 2.22x | 2.29x |

errors: the number of type errors each compiler reports on the project; tsgo's and tsrs's must be equal (a bold errors cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster; bold from 5x). peak memory: maximum resident set size. memory efficiency vs tsgo: tsgo peak memory / tsrs peak memory (2x = tsrs uses half the memory; below 1 = tsrs uses more; bold from 4x). speedup vs bun: bun check wall / tsrs wall; memory efficiency vs bun: bun check peak memory / tsrs peak memory. bun check follows TypeScript 7.0, tsrs the 7.1-dev commit it ports, so their error counts can differ where the two TypeScript versions do.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-16` (16 vCPU, 63 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-09 06:39 UTC. tsrs commit: `2fc4b5051938`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
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
| Output independent of the checker count and assignment | yes, three open cases and the residue of a fourth (tsgo: no) | diagnostics and `.d.ts` identical for `--checkers 1`-16 and 20 random assignments on the error-rich corpora; `--checkerAssignment go` keeps tsgo's history-dependent output for byte-identity (notes/perf-order-independence.md). Open: TanStack/router, sequelize and rxjs still print assignment-dependent output through tsgo's own history dependence (notes/open-history-dependence.md). The TS2590 of #218, which tsgo reports once per checker at the first evaluation, is reported by default once per file, at the first site; the forms that reach a use through a declaration's cached type (a declared constant, alias, constraint or member) are still assignment-dependent (section 4 there). Where that matters, a named assignment and a fixed count (`--checkerAssignment locality --checkers 8`) print the same output in every run |
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
| WebAssembly build (`@maschwenk/tsrs-wasm`: Node, browsers) | yes, single-threaded, released from 0.9.0 | byte-identical to native `--singleThreaded` on 1,998 of 1,998 runnable cases of a 2,000-case conformance sample (also 1,993 of 1,993 over in-memory files), 25/25 regressions, 4 bench projects and emit on 2; 2.58 MB gzip; warm runs 1.8-2.3x native single-threaded on the bench projects (notes/wasm-build.md) |

## Options and defaults

`--extendedDiagnostics` prints the usual counters.
Without `--checkers`, tsrs runs one checker thread per core up to 8 and half as many as the machine has cores above
that, at least tsgo's 4 and at most 32 (8 on 8 cores and on 16, 9 on 18, 32 on 64; 4 for small programs and in `-b`
build mode; up to 0.4.0 the cap was 8, and up to 0.6.0 an 8-core machine got 4); diagnostics do not depend on the
count, the `--extendedDiagnostics` counters and peak memory do.
In a `--noEmit` check, tsrs frees the syntax tree and binder output of a test, spec, story or mock file once it is
checked, when the program gets at most 16 checkers (vscode: -13 to -16% peak memory; `TSRS_FREE_LEAVES=0` turns it
off; notes/mem-free-leaf-files.md).
In a CLI run that type-checks no declaration file (`skipLibCheck` or `noCheck`), the member lists of interfaces,
classes and type literals in declaration files are parsed and bound the first time something reads them, and
the lists of the global libraries (default libs, `types`, `/// <reference types>`) are forced in parallel before
the checkers start (formbricks-web: -7% peak memory at 32 checkers, -12% at 4; `TSRS_LAZY_DTS=0` turns it off;
notes/mem-lazy-dts-members.md).
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

## WebAssembly

`crates/tsrs_wasm` builds tsc as a `wasm32-wasip1` module over a host file system, and `npm/tsrs-wasm` runs it in
Node (a `tsrs-wasm` command on the real file system, or in-memory files with JSON diagnostics) and in browsers
(a Web Worker over in-memory files). It is single-threaded, needs Node 22 or later, and is on npm from 0.9.0:

```sh
npx -y @maschwenk/tsrs-wasm -p .                            # tsc on the real file system, in WebAssembly
```

From source:

```sh
rustup target add wasm32-wasip1 && brew install binaryen   # wasm-opt
tools/wasm/build.sh                                         # writes npm/tsrs-wasm/tsrs.wasm, prints its sizes
node npm/tsrs-wasm/bin/tsrs-wasm.js -p path/to/project
```

```js
import { tsc } from "@maschwenk/tsrs-wasm";
const { exitCode, diagnostics } = await tsc(["-p", "."], { files: { "/tsconfig.json": "{}", "/a.ts": "let x: string = 1;" }, diagnostics: "json" });
```

Its output is byte-identical to native `tsrs --singleThreaded` on the differential gate (`tools/wasm/gate.sh`, run in
CI). Warm runs take 1.8-2.3x native single-threaded time on the bench projects; the module is 9.1 MB, 2.58 MB gzip.
Not supported: `--watch`, `--lsp`, `--api`, JSON diagnostics with `--build`, more than one checker, programs over
4 GiB. [`notes/wasm-build.md`](notes/wasm-build.md) has the design, the stack sizes, the sizes and timings, and the
gate counts; [`npm/tsrs-wasm/README.md`](npm/tsrs-wasm/README.md) the API.

### Against ts-rust's WebAssembly build

[ts-rust](https://github.com/pingdotgg/ts-rust) is another Rust port of the TypeScript 7 compiler with a wasm build. On
nine bench projects, tsrs's module is **2.7x to 7.0x faster** warm and peaks at **1.2x to 2.6x less memory**. Error
counts match on eight; on webpack ts-rust reports 849 errors against 840 (it follows a different TypeScript commit).

| project | native tsrs, one thread | tsrs-wasm | ts-rust wasm | tsrs-wasm faster | peak memory, tsrs-wasm | peak memory, ts-rust wasm | tsrs-wasm less memory |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| xstate-main | 1.01 s | 1.54 s | 4.36 s | **2.82x** | 317 MiB | 627 MiB | **1.98x** |
| webpack | 1.27 s | 2.35 s | 16.33 s | **6.94x** | 448 MiB | 988 MiB | **2.21x** |
| Compiler | 0.18 s | 0.34 s | 1.04 s | **3.04x** | 219 MiB | 253 MiB | **1.16x** |
| Compiler-Unions | 0.34 s | 0.59 s | 1.58 s | **2.67x** | 212 MiB | 254 MiB | **1.20x** |
| next-packages-next | 1.97 s | 3.87 s | 18.78 s | **4.86x** | 560 MiB | 1.27 GiB | **2.32x** |
| storybook | 1.72 s | 2.52 s | 10.40 s | **4.12x** | 415 MiB | 1006 MiB | **2.42x** |
| nuxt | 1.22 s | 2.01 s | 14.02 s | **6.97x** | 405 MiB | 952 MiB | **2.35x** |
| playwright | 0.87 s | 1.56 s | 6.87 s | **4.41x** | 374 MiB | 834 MiB | **2.23x** |
| drizzle-orm | 2.18 s | 3.87 s | 20.72 s | **5.35x** | 537 MiB | 1.38 GiB | **2.63x** |

**Not benchmarked in CI.** Unlike the tables above, which CI re-measures on every push to `main`, this is one run by
hand on an Apple M5 Max (macOS, Node 24) on 2026-10-08 at `2b4fe704`, and nothing re-runs it. Other jobs shared the
machine, so the seconds are inflated and noisy: read the ratios, and expect them to move between runs too (earlier
runs in the note had xstate-main at 3.7-3.8x and webpack at 4.1-5.3x). ts-rust is `pingdotgg/ts-rust` at
`272f79b03881` built with its default `scripts/wasm/build.sh` (opt-level z; its docs list an opt-level s build that
checks 13-18% faster, not tried here), and its module is smaller: 2.08 MB gzip against 2.58 MB. Method, cold times
and ranges: [`notes/wasm-build.md`](notes/wasm-build.md#nine-projects-2026-10-08).

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
- [`notes/wasm-build.md`](notes/wasm-build.md): the WebAssembly build, its gate and its numbers.
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

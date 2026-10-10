# tsrs

[![npm](https://img.shields.io/npm/v/tsrs)](https://www.npmjs.com/package/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

tsrs is a Rust port of the TypeScript 7 compiler and language server. It's a drop-in replacement for `tsc`: same
flags, same tsconfig, same errors.

On the vscode codebase it type-checks **18x faster than tsc 7** using about a third of the memory, and 1.6x faster than
`bun check`. On TypeScript's own test suite it produces the same errors as the reference compiler in all but 4 of
13,462 tests.

## Quick start

```sh
npx -y tsrs -p path/to/project            # compile, like tsc
npx -y tsrs -p path/to/project --noEmit   # type check only
```

Or install it with `pnpm add -D tsrs` and run `tsrs` in place of `tsc`. Prebuilt binaries are available for
macOS arm64 and Linux x64/arm64. There is also a [WebAssembly build](#webassembly) for Node and browsers.

## Performance

Type-checking vscode (10,427 files) on a 64-core Linux machine, each tool with its default settings, averaged over 20
runs ([source](bench/results/compare/2026-10-07-ae33f10b1b8b-64t.md)):

| | time | peak memory |
| --- | ---: | ---: |
| tsc 7 (tsgo 7.0.2) | 9.79 s | 7.52 GiB |
| `bun check` (Bun 1.4.3 canary) | 0.87 s | 2.86 GiB |
| **tsrs** | **0.53 s** | **2.64 GiB** |

The table below covers seventeen projects on a 16-core machine. CI regenerates it on every push to `main`, so expect
the numbers to move by a few percent between runs.

<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2 vs bun check

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), or from a set of large open-source applications (cal-diy, formbricks-web, supabase-studio, t3code-server, mikro-orm, next-packages-next, next-root, storybook, nuxt, playwright, drizzle-orm), with tsgo 7.0.2 (npm `typescript@7.0.2`), with tsrs at commit `c71ed097e9a1` (the PGO-optimized `dist` build, built like the npm release binaries) and with `bun check` from Bun 1.4.4-canary.1+71d0d4399 (one thread per core unless a `--threads` flag is named): `tsc -p <project> --noEmit`, median of 10 interleaved runs (3 for tsgo), on Depot CI `depot-ubuntu-24.04-16` (16 vCPU, 63 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Walls on this shared machine move by a few percent between runs of the same code; the single-threaded instruction counts are the deterministic measure.

**Default mode on a 16-vCPU machine: no thread flag; tsgo uses 4 checker threads, tsrs every core up to 16, then half the cores, at most 32 (16 here; tsrs also resolves members lazily, its default), bun all 16 cores**

| project | errors, tsgo / tsrs / bun | tsgo wall (s) | tsrs wall (s) | bun check wall (s) | speedup vs tsgo | speedup vs bun | tsgo peak memory | tsrs peak memory | bun check peak memory | memory efficiency vs tsgo | memory efficiency vs bun |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| mui-docs | 0 / 0 / 0 | 12.88 | 0.93 | 7.55 | **13.85x** | 8.11x | 7.44 GiB | 1.59 GiB | 11.61 GiB | **4.68x** | 7.30x |
| next-packages-next | 6 / 5 (ref 5) / 6 | 2.85 | 0.22 | 0.37 | **13.10x** | 1.70x | 1.79 GiB | 693 MiB | 749 MiB | 2.64x | 1.08x |
| mikro-orm | 43651 / 43651 / 43651 | 10.00 | 0.81 | 1.85 | **12.27x** | 2.27x | 6.30 GiB | 1.95 GiB | 1.41 GiB | 3.24x | 0.73x |
| vscode | 359 / 371 (ref 371) / 359 | 11.66 | 0.95 | 1.70 | **12.25x** | 1.79x | 7.51 GiB | 2.07 GiB | 1.87 GiB | 3.62x | 0.90x |
| storybook | 93 / 93 / 93 | 1.59 | 0.16 | 0.27 | **10.10x** | 1.72x | 1.38 GiB | 510 MiB | 607 MiB | 2.77x | 1.19x |
| formbricks-web | 0 / 0 / 0 | 7.22 | 0.73 | 1.08 | **9.89x** | 1.48x | 5.82 GiB | 1.96 GiB | 1.61 GiB | 2.97x | 0.82x |
| supabase-studio | 0 / 9 (ref 9) / 0 | 7.16 | 0.76 | 1.74 | **9.47x** | 2.30x | 4.75 GiB | 1.68 GiB | 1.20 GiB | 2.83x | 0.72x |
| t3code-server | 5 / 6 (ref 6) / 5 | 13.52 | 1.49 | 2.75 | **9.08x** | 1.85x | 6.05 GiB | 2.12 GiB | 1.11 GiB | 2.85x | 0.52x |
| webpack | 848 / 840 (ref 840) / 837 | 1.33 | 0.15 | 0.29 | **8.90x** | 1.90x | 1.15 GiB | 533 MiB | 495 MiB | 2.22x | 0.93x |
| drizzle-orm | 10845 / 10846 (ref 10846) / 10845 | 1.72 | 0.20 | 0.40 | **8.77x** | 2.05x | 1.71 GiB | 810 MiB | 733 MiB | 2.16x | 0.90x |
| playwright | 13 / 13 / 13 | 1.11 | 0.13 | 0.22 | **8.56x** | 1.68x | 1.05 GiB | 487 MiB | 475 MiB | 2.21x | 0.98x |
| next-root | 441 / 438 (ref 438) / 441 | 1.49 | 0.18 | 0.28 | **8.28x** | 1.57x | 1.30 GiB | 488 MiB | 569 MiB | 2.73x | 1.16x |
| cal-diy | 136 / 136 / 136 | 5.76 | 0.73 | 1.57 | **7.89x** | 2.16x | 4.32 GiB | 1.86 GiB | 1.30 GiB | 2.32x | 0.70x |
| xstate-main | 0 / 0 / 0 | 0.93 | 0.14 | 0.16 | **6.90x** | 1.21x | 735 MiB | 278 MiB | 393 MiB | 2.64x | 1.42x |
| nuxt | 3 / 4 (ref 4) / 3 | 1.20 | 0.18 | 0.43 | **6.79x** | 2.41x | 1.16 GiB | 519 MiB | 542 MiB | 2.28x | 1.04x |
| Compiler-Unions | 41 / 41 / 41 | 0.52 | 0.13 | 0.21 | 3.93x | 1.55x | 239 MiB | 102 MiB | 228 MiB | 2.35x | 2.25x |
| Compiler | 43 / 43 / 43 | 0.27 | 0.07 | 0.11 | 3.82x | 1.58x | 218 MiB | 99 MiB | 217 MiB | 2.21x | 2.20x |

errors: the number of type errors each compiler reports on the project; tsgo's and tsrs's must be equal (a bold errors cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster; bold from 5x). peak memory: maximum resident set size. memory efficiency vs tsgo: tsgo peak memory / tsrs peak memory (2x = tsrs uses half the memory; below 1 = tsrs uses more; bold from 4x). speedup vs bun: bun check wall / tsrs wall; memory efficiency vs bun: bun check peak memory / tsrs peak memory. bun check follows TypeScript 7.0, tsrs the 7.1-dev commit it ports, so their error counts can differ where the two TypeScript versions do.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-16` (16 vCPU, 63 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-10 15:54 UTC. tsrs commit: `c71ed097e9a1`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
<!-- bench:end -->

### Against bun check at equal thread counts

The tools pick different thread counts by default, so here each one gets the same number (vscode, 64-core machine, 20
runs each):

| threads | tsgo 7.0.2 | tsrs | `bun check` |
| ---: | ---: | ---: | ---: |
| 4 | 9.75 s, 7.43 GiB | 2.43 s, 1.83 GiB | 4.57 s, 1.46 GiB |
| 8 | 6.13 s, 7.99 GiB | 1.35 s, 1.97 GiB | 2.47 s, 1.60 GiB |
| 16 | 4.54 s, 8.67 GiB | 0.81 s, 2.13 GiB | 1.49 s, 1.85 GiB |
| 64 | 4.01 s, 11.58 GiB | 0.44 s, 3.06 GiB | 0.87 s, 2.86 GiB |

tsrs is about 1.9x faster at every thread count; tsrs uses 7-25% more memory. `bun check` follows TypeScript
7.0 and tsrs follows 7.1-dev, so their error counts can differ slightly; each matches its own reference exactly.

[`bench/README.md`](bench/README.md) explains how everything is measured and how to rerun it. The published numbers
use the PGO-optimized release build, which is 7-14% faster than a plain `cargo build --release`.

## Correctness

tsrs is a function-by-function port of the Go implementation of TypeScript (`microsoft/TypeScript`, commit
`b85298b6a81f`, TypeScript 7.1.0-dev). The Go code is the spec: any difference from it in diagnostics, their order,
message text or exit code is a tsrs bug, even when tsrs's output looks better.

On TypeScript's conformance suite, tsrs matches the reference on 13,458 of 13,462 error baselines (the other four are
test-harness artifacts) and on all 12,779 `.types` and `.symbols` baselines. On the 38k-file codebase it reports the
same diagnostics and emits the same 9,257 files byte for byte. [`docs/STATUS.md`](docs/STATUS.md) has the details.

## What it does and doesn't do

"Identical" below means identical to tsgo built from the same pinned commit.

| feature | status | evidence and notes |
| --- | --- | --- |
| Type checking (`--noEmit`) | yes | 13,458 of 13,462 error baselines and all 12,779 `.types` / `.symbols` baselines match; same diagnostics on the 38k-file codebase |
| Multithreaded checking, `--singleThreaded`, `--pretty`, `--extendedDiagnostics`, `--listFiles`, `--listFilesOnly` | yes | |
| Memory target (`--maxMemory <size>`) | `--noEmit` checks, opt-in | the 38k-file codebase at 8 checkers: peak 18.0 GB without it; `--maxMemory 12G` 12.9 GB (+11% instructions), `10G` 10.8 GB (+16%), `8G` 8.7 GB (+35%), same diagnostics; testdata/regressions identical with a checker retired after nearly every file, also in poison mode (notes/mem-recycle-checkers.md) |
| Same output regardless of thread count | yes, with known exceptions (tsgo: no) | a few projects still depend on file order through tsgo's own logic (notes/open-history-dependence.md); `--checkerAssignment go` reproduces tsgo exactly |
| JavaScript emit | yes | all of tsgo's transforms; 13,392 `.js` baselines pass, 0 fail |
| Declaration emit (`.d.ts`) | yes | part of the `.js` baselines |
| Source maps, declaration maps | yes | 149 `.js.map` and 156 `.sourcemap.txt` baselines pass, 0 fail |
| Incremental builds (`.tsbuildinfo`) | yes | identical tsbuildinfo files |
| `--build` (project references) | yes | 189 of 192 `tsbuild` and 191 of 223 `tsc` scenario baselines pass; the failures are CLI commands tsrs doesn't port (`--help`, `--init`, …). Uses more memory than tsgo on large graphs |
| Emit on the 38k-file codebase | yes | all 96 packages: 9,257 output files byte-identical, same diagnostics and exit codes |
| Content mappers (`.vue`, `.svelte`, `.astro` via `contentMappers`) | CLI and `-b`: yes. Language server: no | all 9 content-mapper scenario baselines identical; the 15 content-mapper conformance cases are not run yet (notes/contentmappers.md) |
| Language server (`--lsp -stdio`) | yes | 4,066 of 4,546 fourslash tests pass; 13,869 of 13,869 responses identical in a recorded editor session |
| Automatic type acquisition, telemetry, profiling (language server) | no | not ported |
| Node API server (`--api`) | yes, with gaps | upstream client test suites pass; no profiling requests, no Windows named pipes ([`docs/NODE_API.md`](docs/NODE_API.md)) |
| WebAssembly (`@ts-rs/wasm`) | yes, single-threaded | byte-identical to native `--singleThreaded` on a 2,000-case conformance sample (notes/wasm-build.md) |
| Options TypeScript 7 removed (ES5 target, AMD/UMD/System, `baseUrl`, …) | rejected, as in tsgo | same TS5102 / TS5108 errors |
| `--watch` | no | exits with "not supported" |
| `--init`, `--showConfig` | no | exits with "not supported"; `--help` prints a short usage |
| `--locale` | ignored | messages are English only |
| `--generateTrace` | no | warns "Failed to start tracing" (tsgo's message when it can't write a trace) and type-checks as without the flag |
| Prebuilt binaries | macOS arm64, Linux x64/arm64 (glibc) | no Windows or Intel macOS |

## Options

tsrs accepts every `tsc` flag it supports. A few extra ones control performance:

- `--checkers N` sets the number of checker threads. By default tsrs uses one per core up to 16 cores, then half the
  cores, from 4 to 32; a program with fewer than 32 type-checked files per checker gets fewer, down to 4, and never
  more checkers than files. `--build` uses 4 per project. More threads are faster but use more memory. The thread
  count doesn't change the diagnostics, apart from the known exceptions in the table above.
- `--singleThreaded` uses one thread, for the lowest memory use.
- `--checkerCostCache <file>` remembers how long each file took to check and uses that to balance threads on the next
  run (up to ~15% faster on repeated runs).
- `--maxMemory <size>` (for example `12G`; `--noEmit` checks only) keeps the process near that much memory: above it,
  tsrs replaces its largest checker with a fresh one, trading CPU for memory. Useful on very large programs (the
  38k-file codebase: 18.0 GB without it, 10.8 GB at `10G` for 16% more instructions). The diagnostics are the same.
- `--checkerAssignment go` splits files across threads the way tsgo does, for byte-identical output with tsgo.
- `--noLazyMembers` turns off a set of checker optimizations that are not yet merged upstream. They never change
  diagnostics; the patches are in [`upstream/`](upstream/).

## Language server

`tsrs --lsp -stdio` is a port of the TypeScript 7 language server, with diagnostics, hover, go to definition,
references, rename, completions with auto-imports, code fixes, formatting and the rest. It passes 4,066 of 4,546 of
TypeScript's fourslash tests (most of the rest are skipped by the Go implementation too). Over a 200-edit session on
the 38k-file codebase it stays flat at 2.9 GiB, where the Go server grows from 4.9 to 7.4 GiB.

Neovim 0.11+:

```lua
vim.lsp.config('tsrs', {
  cmd = { 'tsrs', '--lsp', '-stdio' },
  filetypes = { 'typescript', 'typescriptreact', 'javascript', 'javascriptreact' },
  root_markers = { 'tsconfig.json', 'jsconfig.json', 'package.json', '.git' },
})
vim.lsp.enable('tsrs')
```

VS Code: the TypeScript 7 extension runs whatever `tsgo` executable it finds in `js/ts.tsdk.path`. Point that setting
at a directory containing a `tsgo` symlink to the tsrs binary. [`docs/LSP.md`](docs/LSP.md) has the full setup and the
known gaps.

## WebAssembly

`@ts-rs/wasm` runs tsrs in Node 22+ or in a browser. It is single-threaded and its output matches native
`tsrs --singleThreaded` byte for byte.

```sh
npx -y @ts-rs/wasm -p .
```

```js
import { tsc } from "@ts-rs/wasm";
const { exitCode, diagnostics } = await tsc(["-p", "."], { files: { "/tsconfig.json": "{}", "/a.ts": "let x: string = 1;" }, diagnostics: "json" });
```

It runs at about half the speed of native single-threaded tsrs, and the module is 2.58 MB gzipped. In a one-off
comparison it was 2.7x to 7x faster than [ts-rust](https://github.com/pingdotgg/ts-rust)'s WebAssembly build and used
1.2x to 2.6x less memory ([notes/wasm-build.md](notes/wasm-build.md#nine-projects-2026-10-08)). It doesn't support
`--watch`, `--lsp`, `--api` or multiple threads. See [`npm/tsrs-wasm/README.md`](npm/tsrs-wasm/README.md) for the API.

## Build from source

```sh
cargo build --release -p tsrs_cli -p tsrs_testrunner
./target/release/tsrs -p path/to/project
./target/release/tsrs-test run --suite all --baselines types,symbols   # conformance suite, ~20 s
cargo build --release -p tsrs_fourslash && ./target/release/tsrs-fourslash run   # language server tests, ~5 s
tools/wasm/build.sh                                                     # WebAssembly build (needs wasm32-wasip1, wasm-opt)
```

## Docs

- [`CONTRIBUTING.md`](CONTRIBUTING.md): the development workflow and porting rules.
- [`docs/STATUS.md`](docs/STATUS.md): status log and release notes.
- [`docs/EMIT.md`](docs/EMIT.md): JavaScript, declaration and source map emit.
- [`docs/LSP.md`](docs/LSP.md): the language server.
- [`docs/NODE_API.md`](docs/NODE_API.md): the Node API.
- [`docs/DEBUGGING.md`](docs/DEBUGGING.md): tracking down a difference from tsgo.
- [`bench/README.md`](bench/README.md): how the benchmarks work.
- `notes/`: one note per performance experiment, with numbers.

## Contributing

Bug reports with a small reproduction are the most useful contribution, and fixes are welcome. Any difference from tsgo
counts as a bug. See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md) for reporting vulnerabilities,
and the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

A derivative of microsoft/TypeScript (Copyright Microsoft Corporation), licensed Apache-2.0 like the original
([`LICENSE`](LICENSE), [`NOTICE`](NOTICE)). The bundled `lib.*.d.ts` files are TypeScript's. Not affiliated with or
endorsed by Microsoft. The port was written largely by AI coding agents, directed and reviewed by Max Schwenk; its
correctness rests on the test results above rather than on review alone.
